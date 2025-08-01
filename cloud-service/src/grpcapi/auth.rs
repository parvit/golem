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

use crate::auth::AccountAuthorisation;
use crate::grpcapi::get_authorisation_token;
use crate::service::auth::{AuthService, AuthServiceError};
use golem_api_grpc::proto::golem::auth::v1::cloud_auth_service_server::CloudAuthService;
use golem_api_grpc::proto::golem::auth::v1::{
    auth_error, authorize_account_action_response, authorize_project_action_response,
    get_account_response, AuthError, AuthorizeAccountActionRequest, AuthorizeAccountActionResponse,
    AuthorizeAccountActionSuccessResponse, AuthorizeProjectActionRequest,
    AuthorizeProjectActionResponse, AuthorizeProjectActionSuccessResponse, GetAccountRequest,
    GetAccountResponse, GetAccountSuccessResponse,
};
use golem_api_grpc::proto::golem::common::ErrorBody;
use golem_common::metrics::api::TraceErrorKind;
use golem_common::model::ProjectId;
use golem_common::recorded_grpc_api_request;
use golem_common::SafeDisplay;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status};
use tracing::Instrument;

impl From<AuthServiceError> for AuthError {
    fn from(value: AuthServiceError) -> Self {
        let error = match value {
            AuthServiceError::InvalidToken(_)
            | AuthServiceError::RoleMissing { .. }
            | AuthServiceError::AccountOwnershipRequired
            | AuthServiceError::AccountAccessForbidden { .. }
            | AuthServiceError::ProjectActionForbidden { .. }
            | AuthServiceError::ProjectAccessForbidden { .. } => {
                auth_error::Error::Unauthorized(ErrorBody {
                    error: value.to_safe_string(),
                })
            }
            AuthServiceError::InternalTokenServiceError(_)
            | AuthServiceError::InternalRepoError(_) => {
                auth_error::Error::InternalError(ErrorBody {
                    error: value.to_safe_string(),
                })
            }
        };
        AuthError { error: Some(error) }
    }
}

pub struct AuthGrpcApi {
    pub auth_service: Arc<dyn AuthService>,
}

impl AuthGrpcApi {
    async fn auth(&self, metadata: MetadataMap) -> Result<AccountAuthorisation, AuthError> {
        match get_authorisation_token(metadata) {
            Some(t) => self
                .auth_service
                .authorization(&t)
                .await
                .map_err(|e| e.into()),
            None => Err(AuthError {
                error: Some(auth_error::Error::Unauthorized(ErrorBody {
                    error: "Missing token".into(),
                })),
            }),
        }
    }

    async fn get_account(
        &self,
        _request: GetAccountRequest,
        metadata: MetadataMap,
    ) -> Result<GetAccountSuccessResponse, AuthError> {
        let auth = self.auth(metadata).await?;

        Ok(GetAccountSuccessResponse {
            account_id: Some(auth.token.account_id.into()),
        })
    }

    async fn authorize_account_action(
        &self,
        request: AuthorizeAccountActionRequest,
        metadata: MetadataMap,
    ) -> Result<AuthorizeAccountActionSuccessResponse, AuthError> {
        let auth = self.auth(metadata).await?;

        self.auth_service
            .authorize_account_action(
                &auth,
                &request.account_id.unwrap().into(),
                &request.action.try_into().unwrap(),
            )
            .await?;

        Ok(AuthorizeAccountActionSuccessResponse {})
    }

    async fn authorize_project_action(
        &self,
        request: AuthorizeProjectActionRequest,
        metadata: MetadataMap,
    ) -> Result<AuthorizeProjectActionSuccessResponse, AuthError> {
        let auth = self.auth(metadata).await?;

        let result = self
            .auth_service
            .authorize_project_action(
                &auth,
                &ProjectId(request.project_id.unwrap().value.unwrap().into()),
                &request.action.try_into().unwrap(),
            )
            .await?;

        Ok(AuthorizeProjectActionSuccessResponse {
            own_account_id: Some(result.own_account_id.into()),
            project_owner_account_id: Some(result.project_owner_account_id.into()),
        })
    }
}

#[async_trait::async_trait]
impl CloudAuthService for AuthGrpcApi {
    async fn get_account(
        &self,
        request: Request<GetAccountRequest>,
    ) -> Result<Response<GetAccountResponse>, Status> {
        let (m, _, r) = request.into_parts();

        let record = recorded_grpc_api_request!("get_account",);

        let response = match self.get_account(r, m).instrument(record.span.clone()).await {
            Ok(payload) => record.succeed(get_account_response::Result::Success(payload)),
            Err(error) => record.fail(
                get_account_response::Result::Error(error.clone()),
                &AuthTraceErrorKind(&error),
            ),
        };

        Ok(Response::new(GetAccountResponse {
            result: Some(response),
        }))
    }

    async fn authorize_account_action(
        &self,
        request: Request<AuthorizeAccountActionRequest>,
    ) -> Result<Response<AuthorizeAccountActionResponse>, Status> {
        let (m, _, r) = request.into_parts();

        let record = recorded_grpc_api_request!("authorize_account_action",);

        let response = match self
            .authorize_account_action(r, m)
            .instrument(record.span.clone())
            .await
        {
            Ok(payload) => {
                record.succeed(authorize_account_action_response::Result::Success(payload))
            }
            Err(error) => record.fail(
                authorize_account_action_response::Result::Error(error.clone()),
                &AuthTraceErrorKind(&error),
            ),
        };

        Ok(Response::new(AuthorizeAccountActionResponse {
            result: Some(response),
        }))
    }

    async fn authorize_project_action(
        &self,
        request: Request<AuthorizeProjectActionRequest>,
    ) -> Result<Response<AuthorizeProjectActionResponse>, Status> {
        let (m, _, r) = request.into_parts();

        let record = recorded_grpc_api_request!("authorize_project_action",);

        let response = match self
            .authorize_project_action(r, m)
            .instrument(record.span.clone())
            .await
        {
            Ok(payload) => {
                record.succeed(authorize_project_action_response::Result::Success(payload))
            }
            Err(error) => record.fail(
                authorize_project_action_response::Result::Error(error.clone()),
                &AuthTraceErrorKind(&error),
            ),
        };

        Ok(Response::new(AuthorizeProjectActionResponse {
            result: Some(response),
        }))
    }
}

pub struct AuthTraceErrorKind<'a>(pub &'a AuthError);

impl Debug for AuthTraceErrorKind<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl TraceErrorKind for AuthTraceErrorKind<'_> {
    fn trace_error_kind(&self) -> &'static str {
        match &self.0.error {
            None => "None",
            Some(error) => match error {
                auth_error::Error::BadRequest(_) => "BadRequest",
                auth_error::Error::Unauthorized(_) => "Unauthorized",
                auth_error::Error::InternalError(_) => "InternalError",
            },
        }
    }

    fn is_expected(&self) -> bool {
        match &self.0.error {
            None => false,
            Some(error) => match error {
                auth_error::Error::BadRequest(_) => true,
                auth_error::Error::Unauthorized(_) => true,
                auth_error::Error::InternalError(_) => false,
            },
        }
    }
}
