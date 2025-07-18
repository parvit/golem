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

use std::sync::Arc;

use crate::gateway_api_definition::http::CompiledHttpApiDefinition;
use crate::gateway_api_deployment::ApiSiteString;
use crate::service::gateway::api_deployment::{ApiDeploymentError, ApiDeploymentService};
use async_trait::async_trait;
use golem_common::SafeDisplay;
use tracing::error;

// To lookup the set of API Definitions based on an incoming input.
// The input can be HttpRequest or GrpcRequest and so forth, and ApiDefinition
// depends on what is the input. There cannot be multiple types of ApiDefinition
// for a given input type.
#[async_trait]
pub trait HttpApiDefinitionsLookup: Send + Sync {
    async fn get(
        &self,
        host: &ApiSiteString,
    ) -> Result<Vec<CompiledHttpApiDefinition>, ApiDefinitionLookupError>;
}

pub enum ApiDefinitionLookupError {
    ApiDeploymentError(ApiDeploymentError),
    UnknownSite(ApiSiteString),
}

impl SafeDisplay for ApiDefinitionLookupError {
    fn to_safe_string(&self) -> String {
        match self {
            ApiDefinitionLookupError::ApiDeploymentError(err) => err.to_string(),
            ApiDefinitionLookupError::UnknownSite(_) => "Unknown authority".to_string(),
        }
    }
}

pub struct DefaultHttpApiDefinitionLookup {
    deployment_service: Arc<dyn ApiDeploymentService>,
}

impl DefaultHttpApiDefinitionLookup {
    pub fn new(deployment_service: Arc<dyn ApiDeploymentService>) -> Self {
        Self { deployment_service }
    }
}

#[async_trait]
impl HttpApiDefinitionsLookup for DefaultHttpApiDefinitionLookup {
    async fn get(
        &self,
        host: &ApiSiteString,
    ) -> Result<Vec<CompiledHttpApiDefinition>, ApiDefinitionLookupError> {
        let http_api_defs = self
            .deployment_service
            .get_all_definitions_by_site(host)
            .await
            .map_err(|err| {
                error!("Failed to lookup API definitions: {}", err);
                ApiDefinitionLookupError::ApiDeploymentError(err)
            })?;

        if http_api_defs.is_empty() {
            error!("No API definitions found for site: {}", host);
            return Err(ApiDefinitionLookupError::UnknownSite(host.clone()));
        }

        Ok(http_api_defs)
    }
}
