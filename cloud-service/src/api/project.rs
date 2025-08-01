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
use crate::service::api_mapper::ApiMapper;
use crate::service::auth::AuthService;
use crate::service::project::ProjectService;
use futures::{stream, StreamExt, TryStreamExt};
use golem_common::model::auth::{AccountAction, ProjectAction, ProjectPermission};
use golem_common::model::error::ErrorBody;
use golem_common::model::plugin::{PluginInstallationCreation, PluginInstallationUpdate};
use golem_common::model::{Empty, PluginInstallationId, ProjectId};
use golem_common::recorded_http_api_request;
use golem_service_base::api_tags::ApiTags;
use golem_service_base::dto;
use golem_service_base::model::auth::GolemSecurityScheme;
use golem_service_base::model::BatchPluginInstallationUpdates;
use poem_openapi::param::{Path, Query};
use poem_openapi::payload::Json;
use poem_openapi::*;
use std::sync::Arc;
use tracing::Instrument;

pub struct ProjectApi {
    pub auth_service: Arc<dyn AuthService>,
    pub project_service: Arc<dyn ProjectService>,
    pub api_mapper: Arc<ApiMapper>,
}

#[OpenApi(prefix_path = "/v1/projects", tag = ApiTags::Project)]
impl ProjectApi {
    /// Get the default project
    ///
    /// - name of the project can be used for lookup the project if the ID is now known
    /// - defaultEnvironmentId is currently always default
    /// - projectType is either Default
    #[oai(
        path = "/default",
        method = "get",
        operation_id = "get_default_project"
    )]
    async fn get_default_project(&self, token: GolemSecurityScheme) -> ApiResult<Json<Project>> {
        let record = recorded_http_api_request!("get_default_project",);
        let response = self
            .get_default_project_internal(token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn get_default_project_internal(
        &self,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Project>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        let account_id = &auth.token.account_id;
        self.auth_service
            .authorize_account_action(&auth, account_id, &AccountAction::ViewDefaultProject)
            .await?;

        let project = self
            .project_service
            .get_default(&auth.token.account_id)
            .await?;
        Ok(Json(project))
    }

    /// List all projects
    ///
    /// Returns all projects of the account if no project-name is specified.
    /// Otherwise, returns all projects of the account that has the given name.
    /// As unique names are not enforced on the API level, the response may contain multiple entries.
    #[oai(path = "/", method = "get", operation_id = "get_projects")]
    async fn get_projects(
        &self,
        /// Filter by project name
        #[oai(name = "project-name")]
        project_name: Query<Option<String>>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Vec<Project>>> {
        let record =
            recorded_http_api_request!("get_projects", project_name = project_name.0.as_ref(),);
        let response = self
            .get_projects_internal(project_name.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn get_projects_internal(
        &self,
        project_name: Option<String>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Vec<Project>>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        let viewable_projects = self.auth_service.viewable_projects(&auth).await?;

        match project_name {
            Some(project_name) => {
                let projects = self
                    .project_service
                    .get_all_by_name(&project_name, viewable_projects)
                    .await?;
                Ok(Json(projects))
            }
            None => {
                let projects = self.project_service.get_all(viewable_projects).await?;
                Ok(Json(projects))
            }
        }
    }

    /// Create project
    ///
    /// Creates a new project. The ownerAccountId must be the caller's account ID.
    #[oai(path = "/", method = "post", operation_id = "create_project")]
    async fn create_project(
        &self,
        request: Json<ProjectDataRequest>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Project>> {
        let record = recorded_http_api_request!("create_project", project_name = request.0.name,);
        let response = self
            .create_project_internal(request.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn create_project_internal(
        &self,
        request: ProjectDataRequest,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Project>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;

        let project = Project {
            project_id: ProjectId::new_v4(),
            project_data: ProjectData {
                name: request.name,
                owner_account_id: request.owner_account_id,
                description: request.description,
                default_environment_id: "default".to_string(),
                project_type: ProjectType::NonDefault,
            },
        };

        self.auth_service
            .authorize_account_action(
                &auth,
                &project.project_data.owner_account_id,
                &AccountAction::CreateProject,
            )
            .await?;

        self.project_service.create(&project).await?;
        Ok(Json(project))
    }

    /// Get project by ID
    ///
    /// Gets a project by its identifier. Response is the same as for the default project.
    #[oai(path = "/:project_id", method = "get", operation_id = "get_project")]
    async fn get_project(
        &self,
        project_id: Path<ProjectId>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Project>> {
        let record =
            recorded_http_api_request!("get_project", project_id = project_id.0.to_string(),);
        let response = self
            .get_project_internal(project_id.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn get_project_internal(
        &self,
        project_id: ProjectId,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Project>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::ViewProject)
            .await?;

        let project = self.project_service.get(&project_id).await?;
        match project {
            Some(p) => Ok(Json(p)),
            None => Err(ApiError::NotFound(Json(ErrorBody {
                error: "Project not found".to_string(),
            }))),
        }
    }

    /// Delete project
    ///
    /// Deletes a project given by its identifier.
    #[oai(
        path = "/:project_id",
        method = "delete",
        operation_id = "delete_project"
    )]
    async fn delete_project(
        &self,
        project_id: Path<ProjectId>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<DeleteProjectResponse>> {
        let record =
            recorded_http_api_request!("delete_project", project_id = project_id.0.to_string(),);
        let response = self
            .delete_project_internal(project_id.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn delete_project_internal(
        &self,
        project_id: ProjectId,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<DeleteProjectResponse>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::DeleteProject)
            .await?;

        self.project_service.delete(&project_id).await?;
        Ok(Json(DeleteProjectResponse {}))
    }

    /// Get project actions
    ///
    /// Returns a list of actions that can be performed on the project.
    #[oai(
        path = "/:project_id/actions",
        method = "get",
        operation_id = "get_project_actions"
    )]
    async fn get_project_actions(
        &self,
        project_id: Path<ProjectId>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Vec<ProjectPermission>>> {
        let record = recorded_http_api_request!(
            "get_project_actions",
            project_id = project_id.0.to_string(),
        );
        let response = self
            .get_project_actions_internal(project_id.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn get_project_actions_internal(
        &self,
        project_id: ProjectId,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Vec<ProjectPermission>>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        let result = self
            .auth_service
            .get_project_actions(&auth, &project_id)
            .await?;
        Ok(Json(Vec::from_iter(result.actions.actions)))
    }

    /// Gets the list of plugins installed for the given project
    #[oai(
        path = "/:project_id/plugins/installs",
        method = "get",
        operation_id = "get_installed_plugins_of_project"
    )]
    async fn get_installed_plugins(
        &self,
        project_id: Path<ProjectId>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Vec<dto::PluginInstallation>>> {
        let record = recorded_http_api_request!(
            "get_installed_plugins_of_project",
            project_id = project_id.0.to_string(),
        );

        let response = self
            .get_installed_plugins_internal(project_id.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn get_installed_plugins_internal(
        &self,
        project_id: ProjectId,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Vec<dto::PluginInstallation>>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::ViewPluginInstallations)
            .await?;

        let response = self
            .project_service
            .get_plugin_installations_for_project(&project_id)
            .await?;

        let secret = &token.secret();
        let converted = stream::iter(response)
            .then(|pi| self.api_mapper.convert_plugin_installation(secret, pi))
            .try_collect::<Vec<_>>()
            .await?;

        Ok(Json(converted))
    }

    /// Installs a new plugin for this project
    #[oai(
        path = "/:project_id/plugins/installs",
        method = "post",
        operation_id = "install_plugin_to_project"
    )]
    async fn install_plugin(
        &self,
        project_id: Path<ProjectId>,
        plugin: Json<PluginInstallationCreation>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<dto::PluginInstallation>> {
        let record = recorded_http_api_request!(
            "install_plugin",
            project_id = project_id.0.to_string(),
            plugin_name = plugin.name.clone(),
            plugin_version = plugin.version.clone()
        );

        let response = self
            .install_plugin_internal(project_id.0, plugin.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn install_plugin_internal(
        &self,
        project_id: ProjectId,
        plugin: PluginInstallationCreation,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<dto::PluginInstallation>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::CreatePluginInstallation)
            .await?;

        let token = token.secret();

        let plugin_installation = self
            .project_service
            .create_plugin_installation_for_project(&project_id, plugin, &token)
            .await?;

        Ok(Json(
            self.api_mapper
                .convert_plugin_installation(&token, plugin_installation)
                .await?,
        ))
    }

    /// Updates the priority or parameters of a plugin installation
    #[oai(
        path = "/:project_id/plugins/installs/:installation_id",
        method = "put",
        operation_id = "update_installed_plugin_in_project"
    )]
    async fn update_installed_plugin(
        &self,
        project_id: Path<ProjectId>,
        installation_id: Path<PluginInstallationId>,
        update: Json<PluginInstallationUpdate>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Empty>> {
        let record = recorded_http_api_request!(
            "update_installed_plugin",
            project_id = project_id.0.to_string(),
            installation_id = installation_id.0.to_string()
        );

        let response = self
            .update_installed_plugins_internal(project_id.0, installation_id.0, update.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn update_installed_plugins_internal(
        &self,
        project_id: ProjectId,
        installation_id: PluginInstallationId,
        update: PluginInstallationUpdate,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Empty>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::UpdatePluginInstallation)
            .await?;

        let token = token.secret();

        self.project_service
            .update_plugin_installation_for_project(&project_id, &installation_id, update, &token)
            .await
            .map_err(|e| e.into())
            .map(|_| Json(Empty {}))
    }

    /// Uninstalls a plugin from this project
    #[oai(
        path = "/:project_id/latest/plugins/installs/:installation_id",
        method = "delete",
        operation_id = "uninstall_plugin_from_project"
    )]
    async fn uninstall_plugin(
        &self,
        project_id: Path<ProjectId>,
        installation_id: Path<PluginInstallationId>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Empty>> {
        let record = recorded_http_api_request!(
            "uninstall_plugin",
            project_id = project_id.0.to_string(),
            installation_id = installation_id.0.to_string()
        );

        let response = self
            .uninstall_plugin_internal(project_id.0, installation_id.0, token)
            .instrument(record.span.clone())
            .await;

        record.result(response)
    }

    async fn uninstall_plugin_internal(
        &self,
        project_id: ProjectId,
        installation_id: PluginInstallationId,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Empty>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(&auth, &project_id, &ProjectAction::DeletePluginInstallation)
            .await?;

        let token = token.secret();

        self.project_service
            .delete_plugin_installation_for_project(&installation_id, &project_id, &token)
            .await
            .map_err(|e| e.into())
            .map(|_| Json(Empty {}))
    }

    /// Applies a batch of changes to the installed plugins of a component
    #[oai(
        path = "/:project_id/latest/plugins/installs/batch",
        method = "post",
        operation_id = "batch_update_installed_plugins_of_project"
    )]
    async fn batch_update_installed_plugins(
        &self,
        project_id: Path<ProjectId>,
        updates: Json<BatchPluginInstallationUpdates>,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Empty>> {
        let record = recorded_http_api_request!(
            "batch_update_installed_plugins",
            project_id = project_id.0.to_string(),
        );

        let response = self
            .batch_update_installed_plugins_internal(project_id.0, updates.0, token)
            .instrument(record.span.clone())
            .await;
        record.result(response)
    }

    async fn batch_update_installed_plugins_internal(
        &self,
        project_id: ProjectId,
        updates: BatchPluginInstallationUpdates,
        token: GolemSecurityScheme,
    ) -> ApiResult<Json<Empty>> {
        let auth = self.auth_service.authorization(token.as_ref()).await?;
        self.auth_service
            .authorize_project_action(
                &auth,
                &project_id,
                &ProjectAction::BatchUpdatePluginInstallations,
            )
            .await?;

        let token = token.secret();

        self.project_service
            .batch_update_plugin_installations_for_project(&project_id, &updates.actions, &token)
            .await?;
        Ok(Json(Empty {}))
    }
}
