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

use super::{ApiError, ApiResult};
use crate::model::*;
use crate::service::account::AccountService;
use crate::service::auth::{AuthService, ViewableAccounts};
use crate::service::project_grant::ProjectGrantService;
use crate::service::project_policy::ProjectPolicyService;
use golem_common::model::auth::{ProjectAction, ProjectActions};
use golem_common::model::error::{ErrorBody, ErrorsBody};
use golem_common::model::ProjectId;
use golem_common::model::{ProjectGrantId, ProjectPolicyId};
use golem_common::recorded_http_api_request;
use golem_service_base::api_tags::ApiTags;
use golem_service_base::model::auth::GolemSecurityScheme;
use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::*;
use std::sync::Arc;
use tracing::info;
use tracing::Instrument;

pub struct ProjectGrantApi {
    pub auth_service: Arc<dyn AuthService>,
    pub account_service: Arc<dyn AccountService>,
    pub project_grant_service: Arc<dyn ProjectGrantService>,
    pub project_policy_service: Arc<dyn ProjectPolicyService>,
}

#[OpenApi(prefix_path = "/v1/projects", tag = ApiTags::ProjectGrant)]
impl ProjectGrantApi {
    /// Get a project's grants
    ///
    /// Returns all projects grants associated with the given project.
    ///
    /// For each grant:
    /// - `id`` is the identifier of the grant itself
    /// - `granteeAccountId` the account that gets access for the project
    /// - `grantorProjectId` the project ID
    /// - `projectPolicyId` the associated policy - see the project policy API below
    #[oai(
        path = "/:project_id/grants",
        method = "get",
        operation_id = "get_project_grants"
    )]
    async fn get_project_grants(
        &self,
        project_id: Path<ProjectId>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Vec<ProjectGrant>>> {
        let record = recorded_http_api_request!(
            "get_project_grants",
            project_id = project_id.0.to_string(),
        );
        let response = self
            .get_project_grants_internal(project_id.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn get_project_grants_internal(
        &self,
        project_id: ProjectId,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Vec<ProjectGrant>>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::ViewProjectGrants)
            .await?;

        let grants = self
            .project_grant_service
            .get_by_project(&project_id)
            .await?;
        Ok(Json(grants))
    }

    /// Get a specific grant of a project
    ///
    /// Returns a specific grant of a specific project. The returned object is the same as the elements of the grants endpoint's response described above.
    #[oai(
        path = "/:project_id/grants/:grant_id",
        method = "get",
        operation_id = "get_project_grant"
    )]
    async fn get_project_grant(
        &self,
        project_id: Path<ProjectId>,
        grant_id: Path<ProjectGrantId>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<ProjectGrant>> {
        let record = recorded_http_api_request!(
            "get_project_grant",
            project_id = project_id.0.to_string(),
            project_grant_id = grant_id.0.to_string()
        );
        let response = self
            .get_project_grant_internal(project_id.0, grant_id.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn get_project_grant_internal(
        &self,
        project_id: ProjectId,
        grant_id: ProjectGrantId,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<ProjectGrant>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::ViewProjectGrants)
            .await?;

        let grant = self
            .project_grant_service
            .get(&project_id, &grant_id)
            .await?;

        match grant {
            Some(grant) => Ok(Json(grant)),
            None => Err(ApiError::NotFound(Json(ErrorBody {
                error: "Project grant not found".to_string(),
            }))),
        }
    }

    /// Create a project grant
    ///
    /// Creates a new project grant from the following information:
    /// - `granteeAccountId` the account that gets access for the project
    /// - `projectPolicyId` the associated policy - see the project policy API below
    ///
    /// The response describes the new project grant including its id that can be used to query specifically this grant in the future.
    #[oai(
        path = "/:project_id/grants",
        method = "post",
        operation_id = "create_project_grant"
    )]
    async fn post_project_grant(
        &self,
        project_id: Path<ProjectId>,
        request: Json<ProjectGrantDataRequest>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<ProjectGrant>> {
        let record = recorded_http_api_request!(
            "create_project_grant",
            project_id = project_id.0.to_string()
        );
        let response = self
            .post_project_grant_internal(project_id.0, request.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn post_project_grant_internal(
        &self,
        project_id: ProjectId,
        request: ProjectGrantDataRequest,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<ProjectGrant>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::CreateProjectGrants)
            .await?;

        let account_id = match (request.grantee_account_id, request.grantee_email) {
            (Some(account_id), _) => account_id,
            (None, Some(email)) => {
                info!("Looking up account by email {email}");
                let mut accounts = self
                    .account_service
                    .find(Some(&email), ViewableAccounts::All)
                    .await?;
                if accounts.len() == 1 {
                    accounts.swap_remove(0).id
                } else {
                    Err(ApiError::NotFound(Json(ErrorBody {
                        error: "No matching account found".to_string(),
                    })))?
                }
            }
            (None, None) => Err(ApiError::BadRequest(Json(ErrorsBody {
                errors: vec!["Account id or email need to be provided".to_string()],
            })))?,
        };

        let data = match request.project_policy_id {
            Some(project_policy_id) => ProjectGrantData {
                grantee_account_id: account_id,
                grantor_project_id: project_id,
                project_policy_id,
            },
            None => {
                let policy = ProjectPolicy {
                    id: ProjectPolicyId::new_v4(),
                    name: request.project_policy_name.unwrap_or("".to_string()),
                    project_actions: ProjectActions {
                        actions: request.project_actions.into_iter().collect(),
                    },
                };

                self.project_policy_service.create(&policy).await?;

                ProjectGrantData {
                    grantee_account_id: account_id,
                    grantor_project_id: project_id,
                    project_policy_id: policy.id,
                }
            }
        };

        let grant = ProjectGrant {
            id: ProjectGrantId::new_v4(),
            data,
        };

        self.project_grant_service.create(&grant).await?;
        Ok(Json(grant))
    }

    /// Delete a project grant
    ///
    /// Deletes an existing grant of a specific project.
    #[oai(
        path = "/:project_id/grants/:grant_id",
        method = "delete",
        operation_id = "delete_project_grant"
    )]
    async fn delete_project_grant(
        &self,
        project_id: Path<ProjectId>,
        grant_id: Path<ProjectGrantId>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<DeleteProjectGrantResponse>> {
        let record = recorded_http_api_request!(
            "delete_project_grant",
            project_id = project_id.0.to_string(),
            project_grant_id = grant_id.0.to_string()
        );
        let response = self
            .delete_project_grant_internal(project_id.0, grant_id.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn delete_project_grant_internal(
        &self,
        project_id: ProjectId,
        grant_id: ProjectGrantId,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<DeleteProjectGrantResponse>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::DeleteProjectGrants)
            .await?;

        self.project_grant_service
            .delete(&project_id, &grant_id)
            .await?;

        Ok(Json(DeleteProjectGrantResponse {}))
    }
}
