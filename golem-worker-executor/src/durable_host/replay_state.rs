// Copyright 2024-2025 Golem Cloud
//
// Licensed under the Golem Source License v1.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://license.golem.cloud/LICENSE
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::services::oplog::{Oplog, OplogOps, OplogService};
use golem_common::model::invocation_context::InvocationContextStack;
use golem_common::model::oplog::{
    AtomicOplogIndex, LogLevel, OplogEntry, OplogIndex, PersistenceLevel,
};
use golem_common::model::regions::{DeletedRegions, OplogRegion};
use golem_common::model::{ComponentVersion, IdempotencyKey, OwnedWorkerId};
use golem_service_base::error::worker_executor::WorkerExecutorError;
use golem_wasm_rpc::{Value, ValueAndType};
use metrohash::MetroHash128;
use std::collections::HashSet;
use std::hash::Hasher;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Debug, Clone)]
pub enum ReplayEvent {
    ReplayFinished,
    UpdateReplayed { new_version: ComponentVersion },
}

#[derive(Debug, Clone)]
pub struct ExportedFunctionInvoked {
    pub function_name: String,
    pub function_input: Vec<Value>,
    pub idempotency_key: IdempotencyKey,
    pub invocation_context: InvocationContextStack,
}

#[derive(Clone)]
pub struct ReplayState {
    owned_worker_id: OwnedWorkerId,
    oplog_service: Arc<dyn OplogService>,
    oplog: Arc<dyn Oplog>,
    replay_target: AtomicOplogIndex,
    /// The oplog index of the last replayed entry
    last_replayed_index: AtomicOplogIndex,
    internal: Arc<RwLock<InternalReplayState>>,
    has_seen_logs: Arc<AtomicBool>,
}

#[derive(Clone)]
struct InternalReplayState {
    pub skipped_regions: DeletedRegions,
    pub next_skipped_region: Option<OplogRegion>,
    /// Hashes of log entries persisted since the last read non-hint oplog entry
    pub log_hashes: HashSet<(u64, u64)>,
    /// Updates that were encountered while reading the oplog
    pub pending_replay_events: Vec<ReplayEvent>,
}

impl ReplayState {
    pub async fn new(
        owned_worker_id: OwnedWorkerId,
        oplog_service: Arc<dyn OplogService>,
        oplog: Arc<dyn Oplog>,
        skipped_regions: DeletedRegions,
        last_oplog_index: OplogIndex,
    ) -> Self {
        let next_skipped_region = skipped_regions.find_next_deleted_region(OplogIndex::NONE);
        let mut result = Self {
            owned_worker_id,
            oplog_service,
            oplog,
            last_replayed_index: AtomicOplogIndex::from_oplog_index(OplogIndex::NONE),
            replay_target: AtomicOplogIndex::from_oplog_index(last_oplog_index),
            internal: Arc::new(RwLock::new(InternalReplayState {
                skipped_regions,
                next_skipped_region,
                log_hashes: HashSet::new(),
                pending_replay_events: Vec::new(),
            })),
            has_seen_logs: Arc::new(AtomicBool::new(false)),
        };
        result.move_replay_idx(OplogIndex::INITIAL).await; // By this we handle initial skipped regions applied by manual updates correctly
        result.skip_forward().await;
        result
    }

    pub async fn switch_to_live(&mut self) {
        if !self.is_live() {
            self.record_replay_event(ReplayEvent::ReplayFinished).await;
        }
        self.last_replayed_index.set(self.replay_target.get());
    }

    pub fn last_replayed_index(&self) -> OplogIndex {
        self.last_replayed_index.get()
    }

    pub fn replay_target(&self) -> OplogIndex {
        self.replay_target.get()
    }

    pub fn set_replay_target(&mut self, new_target: OplogIndex) {
        self.replay_target.set(new_target)
    }

    pub async fn skipped_regions(&self) -> DeletedRegions {
        let internal = self.internal.read().await;
        internal.skipped_regions.clone()
    }

    pub async fn add_skipped_region(&mut self, region: OplogRegion) {
        let mut internal = self.internal.write().await;
        internal.skipped_regions.add(region);
    }

    pub async fn is_in_skipped_region(&self, oplog_index: OplogIndex) -> bool {
        let internal = self.internal.read().await;
        internal.skipped_regions.is_in_deleted_region(oplog_index)
    }

    /// Returns whether we are in live mode where we are executing new calls.
    pub fn is_live(&self) -> bool {
        self.last_replayed_index.get() == self.replay_target.get()
    }

    /// Returns whether we are in replay mode where we are replaying old calls.
    pub fn is_replay(&self) -> bool {
        !self.is_live()
    }

    async fn record_replay_event(&mut self, event: ReplayEvent) {
        self.internal
            .write()
            .await
            .pending_replay_events
            .push(event)
    }

    pub async fn take_new_replay_events(&mut self) -> Vec<ReplayEvent> {
        std::mem::take(&mut self.internal.write().await.pending_replay_events)
    }

    /// Reads the next oplog entry, and skips every hint entry following it.
    /// Returns the oplog index of the entry read, no matter how many more hint entries
    /// were read.
    pub async fn get_oplog_entry(&mut self) -> (OplogIndex, OplogEntry) {
        self.try_get_oplog_entry(|_| true).await.unwrap()
    }

    /// Checks whether the currently read `entry` is a hint entry is valid for replay, or
    /// if a new oplog index should be tried instead.
    ///
    /// For hint entries, the next tried oplog index is the next one. When reaching
    /// persist-nothing zones, it points to the end of the zone.
    ///
    /// If the entry is a hint entry, the result is `Some` and contains the current last
    /// read index, so the next read will get the next one.
    /// If the entry is the beginning of a persist-nothing zone, the result will be `Some`
    /// containing the _end_ of the zone so the next read will get the first entry outside
    /// the zone.
    /// If the entry is not a hint entry the result is `None`.
    ///
    async fn should_skip_to(&self, entry: &OplogEntry) -> Option<OplogIndex> {
        if entry.is_hint() {
            // Keeping the last replayed index as-is, so the next attempt will read the next one
            Some(self.last_replayed_index())
        } else if let OplogEntry::ChangePersistenceLevel { level, .. } = &entry {
            if level == &PersistenceLevel::PersistNothing {
                let begin_index = self.last_replayed_index();
                let end_index = self
                    .lookup_oplog_entry(begin_index, |entry, _idx| match entry {
                        OplogEntry::ChangePersistenceLevel { level, .. } => {
                            level != &PersistenceLevel::PersistNothing
                        }
                        OplogEntry::ExportedFunctionCompleted { .. } => true,
                        _ => false,
                    })
                    .await;

                if let Some(end_index) = end_index {
                    Some(end_index)
                } else {
                    // The zone has not been closed
                    Some(self.replay_target())
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Reads the next oplog entry, and if it matches the given condition, skips
    /// every hint entry following it and returns the oplog index of the entry read.
    /// If the condition is not met, returns None and the current replay state remains
    /// unchanged.
    ///
    /// The auto-skipped hint entries can be of two kind:
    /// - A set of oplog entry cases are always hint entries. They manipulate the worker status
    ///   but are non-deterministic from the replay's point of view.
    /// - Every oplog entry recorded in persist-nothing zones. These are there for observability,
    ///   but they never participate in the replay. A persist-nothing zone is bounded by two
    ///   ChangePersistenceLevel entries, or if the closing one is missing, it is up to the end of the
    ///   oplog.
    pub async fn try_get_oplog_entry(
        &mut self,
        condition: impl FnOnce(&OplogEntry) -> bool,
    ) -> Option<(OplogIndex, OplogEntry)> {
        let saved_replay_idx = self.last_replayed_index.get();
        let saved_next_skipped_region = {
            let internal = self.internal.read().await;
            internal.next_skipped_region.clone()
        };

        let read_idx = self.last_replayed_index.get().next();
        let entry = self.internal_get_next_oplog_entry().await;

        if condition(&entry) {
            self.skip_forward().await;

            Some((read_idx, entry))
        } else {
            self.last_replayed_index.set(saved_replay_idx);
            let mut internal = self.internal.write().await;
            internal.next_skipped_region = saved_next_skipped_region;

            None
        }
    }

    async fn skip_forward(&mut self) {
        // Skipping hint entries and recording log entries
        let mut logs = HashSet::new();
        while self.is_replay() {
            let saved_replay_idx = self.last_replayed_index.get();
            let saved_next_skipped_region = {
                let internal = self.internal.read().await;
                internal.next_skipped_region.clone()
            };
            let entry = self.internal_get_next_oplog_entry().await;
            match self.should_skip_to(&entry).await {
                Some(last_read_idx) => {
                    // Recording seen log entries
                    if let OplogEntry::Log {
                        level,
                        context,
                        message,
                        ..
                    } = &entry
                    {
                        let hash = Self::hash_log_entry(*level, context, message);
                        logs.insert(hash);
                    }

                    // Moving the replay pointer
                    self.last_replayed_index.set(last_read_idx);
                    // TODO: what to do with next_skipped_region if we jumped forward to end of persist-nothing zone?
                }
                None => {
                    // We've found the first non-hint entry after the first read one,
                    // so we move everything back the last position (saved_replay_idx), including
                    // possibly skipped regions.
                    self.last_replayed_index.set(saved_replay_idx);
                    let mut internal = self.internal.write().await;
                    // TODO: cache the last hint entry to avoid reading it again
                    internal.next_skipped_region = saved_next_skipped_region;
                    break;
                }
            }
        }

        self.has_seen_logs
            .store(!logs.is_empty(), Ordering::Relaxed);
        let mut internal = self.internal.write().await;
        internal.log_hashes = logs;
    }

    /// Returns true if the given log entry has been seen since the last non-hint oplog entry.
    pub async fn seen_log(&self, level: LogLevel, context: &str, message: &str) -> bool {
        if self.has_seen_logs.load(Ordering::Relaxed) {
            let hash = Self::hash_log_entry(level, context, message);
            let internal = self.internal.read().await;
            internal.log_hashes.contains(&hash)
        } else {
            false
        }
    }

    /// Removes a seen log from the set. If the set becomes empty, `seen_log` becomes a cheap operation
    pub async fn remove_seen_log(&self, level: LogLevel, context: &str, message: &str) {
        let hash = Self::hash_log_entry(level, context, message);
        let mut internal = self.internal.write().await;
        internal.log_hashes.remove(&hash);
        self.has_seen_logs
            .store(!internal.log_hashes.is_empty(), Ordering::Relaxed);
    }

    fn hash_log_entry(level: LogLevel, context: &str, message: &str) -> (u64, u64) {
        let mut hasher = MetroHash128::new();
        hasher.write_u8(level as u8);
        hasher.write(context.as_bytes());
        hasher.write(message.as_bytes());
        hasher.finish128()
    }

    /// Gets the next oplog entry, no matter if it is hint or not, following jumps
    async fn internal_get_next_oplog_entry(&mut self) -> OplogEntry {
        let read_idx = self.last_replayed_index.get().next();

        let oplog_entries = self.read_oplog(read_idx, 1).await;
        let oplog_entry = oplog_entries.into_iter().next().unwrap();

        // record side effects that need to be applied at the next opportunity
        if let OplogEntry::SuccessfulUpdate { target_version, .. } = oplog_entry {
            self.record_replay_event(ReplayEvent::UpdateReplayed {
                new_version: target_version,
            })
            .await
        }

        if read_idx == self.replay_target.get() {
            self.record_replay_event(ReplayEvent::ReplayFinished).await
        }

        self.move_replay_idx(read_idx).await;

        oplog_entry
    }

    async fn move_replay_idx(&mut self, new_idx: OplogIndex) {
        self.last_replayed_index.set(new_idx);
        self.get_out_of_skipped_region().await;
    }

    pub async fn lookup_oplog_entry(
        &self,
        begin_idx: OplogIndex,
        check: impl Fn(&OplogEntry, OplogIndex) -> bool,
    ) -> Option<OplogIndex> {
        self.lookup_oplog_entry_with_condition(begin_idx, check, |_, _| true)
            .await
    }

    pub async fn lookup_oplog_entry_with_condition(
        &self,
        begin_idx: OplogIndex,
        end_check: impl Fn(&OplogEntry, OplogIndex) -> bool,
        for_all_intermediate: impl Fn(&OplogEntry, OplogIndex) -> bool,
    ) -> Option<OplogIndex> {
        let replay_target = self.replay_target.get();
        let mut start = self.last_replayed_index.get().next();

        const CHUNK_SIZE: u64 = 1024;

        let mut current_next_skip_region = self.internal.read().await.next_skipped_region.clone();

        while start < replay_target {
            let entries = self
                .oplog_service
                .read(&self.owned_worker_id, start, CHUNK_SIZE)
                .await;
            for (idx, entry) in &entries {
                if current_next_skip_region
                    .as_ref()
                    .map(|r| r.contains(*idx))
                    .unwrap_or(false)
                {
                    // If we are in the current skip region, ignore the entry
                    continue;
                }
                if current_next_skip_region
                    .as_ref()
                    .map(|r| &r.end == idx)
                    .unwrap_or(false)
                {
                    // if we are at the end of the current skip region, find the next one
                    current_next_skip_region = self
                        .internal
                        .read()
                        .await
                        .skipped_regions
                        .find_next_deleted_region(idx.next());
                }
                if end_check(entry, begin_idx) {
                    return Some(*idx);
                } else if !for_all_intermediate(entry, begin_idx) {
                    return None;
                }
            }
            start = start.range_end(entries.len() as u64).next();
        }

        None
    }

    // TODO: can we rewrite this on top of get_oplog_entry?
    pub async fn get_oplog_entry_exported_function_invoked(
        &mut self,
    ) -> Result<Option<ExportedFunctionInvoked>, WorkerExecutorError> {
        loop {
            if self.is_replay() {
                let (_, oplog_entry) = self.get_oplog_entry().await;
                match &oplog_entry {
                    OplogEntry::ExportedFunctionInvoked {
                        function_name,
                        idempotency_key,
                        trace_id,
                        trace_states: trace_state,
                        invocation_context: spans,
                        ..
                    } => {
                        let request: Vec<golem_wasm_rpc::protobuf::Val> = self
                            .oplog
                            .get_payload_of_entry(&oplog_entry)
                            .await
                            .expect("failed to deserialize function request payload")
                            .unwrap();
                        let request = request
                            .into_iter()
                            .map(|val| {
                                val.try_into()
                                    .expect("failed to decode serialized protobuf value")
                            })
                            .collect::<Vec<Value>>();

                        let invocation_context =
                            InvocationContextStack::from_oplog_data(trace_id, trace_state, spans);

                        break Ok(Some(ExportedFunctionInvoked {
                            function_name: function_name.to_string(),
                            function_input: request,
                            idempotency_key: idempotency_key.clone(),
                            invocation_context,
                        }));
                    }
                    entry if entry.is_hint() => {}
                    _ => {
                        break Err(WorkerExecutorError::unexpected_oplog_entry(
                            "ExportedFunctionInvoked",
                            format!("{oplog_entry:?}"),
                        ));
                    }
                }
            } else {
                break Ok(None);
            }
        }
    }

    // TODO: can we rewrite this on top of get_oplog_entry?
    pub async fn get_oplog_entry_exported_function_completed(
        &mut self,
    ) -> Result<Option<Option<ValueAndType>>, WorkerExecutorError> {
        loop {
            if self.is_replay() {
                let (_, oplog_entry) = self.get_oplog_entry().await;
                match &oplog_entry {
                    OplogEntry::ExportedFunctionCompleted { .. } => {
                        let response: Option<ValueAndType> = self
                            .oplog
                            .get_payload_of_entry(&oplog_entry)
                            .await
                            .expect("failed to deserialize function response payload")
                            .unwrap();

                        break Ok(Some(response));
                    }
                    entry if entry.is_hint() => {}
                    _ => {
                        break Err(WorkerExecutorError::unexpected_oplog_entry(
                            "ExportedFunctionCompleted",
                            format!("{oplog_entry:?}"),
                        ));
                    }
                }
            } else {
                break Ok(None);
            }
        }
    }

    pub(crate) async fn get_out_of_skipped_region(&mut self) {
        if self.is_replay() {
            let mut internal = self.internal.write().await;
            let update_next_skipped_region = match &internal.next_skipped_region {
                Some(region) if region.start == (self.last_replayed_index.get().next()) => {
                    let target = region.end.next(); // we want to continue reading _after_ the region
                    debug!(
                        "Worker reached skipped region at {}, jumping to {} (oplog size: {})",
                        region.start,
                        target,
                        self.replay_target.get()
                    );
                    self.last_replayed_index.set(target.previous()); // so we set the last replayed index to the end of the region

                    true
                }
                _ => false,
            };

            if update_next_skipped_region {
                internal.next_skipped_region = internal
                    .skipped_regions
                    .find_next_deleted_region(self.last_replayed_index.get());
            }
        }
    }

    async fn read_oplog(&self, idx: OplogIndex, n: u64) -> Vec<OplogEntry> {
        self.oplog_service
            .read(&self.owned_worker_id, idx, n)
            .await
            .into_values()
            .collect()
    }
}
