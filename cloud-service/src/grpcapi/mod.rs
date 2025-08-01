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

use crate::bootstrap::Services;
use crate::grpcapi::account::AccountGrpcApi;
use crate::grpcapi::limits::LimitsGrpcApi;
use crate::grpcapi::project::ProjectGrpcApi;
use crate::grpcapi::token::TokenGrpcApi;
use auth::AuthGrpcApi;
use futures::TryFutureExt;
use golem_api_grpc::proto::golem::account::v1::cloud_account_service_server::CloudAccountServiceServer;
use golem_api_grpc::proto::golem::auth::v1::cloud_auth_service_server::CloudAuthServiceServer;
use golem_api_grpc::proto::golem::limit::v1::cloud_limits_service_server::CloudLimitsServiceServer;
use golem_api_grpc::proto::golem::project::v1::cloud_project_service_server::CloudProjectServiceServer;
use golem_api_grpc::proto::golem::token::v1::cloud_token_service_server::CloudTokenServiceServer;
use golem_common::model::auth::TokenSecret as ModelTokenSecret;
use std::net::SocketAddr;
use std::str::FromStr;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::codec::CompressionEncoding;
use tonic::metadata::MetadataMap;
use tonic::transport::Server;
use tracing::Instrument;

mod account;
mod auth;
mod limits;
mod project;
mod token;

pub fn get_authorisation_token(metadata: MetadataMap) -> Option<ModelTokenSecret> {
    let auth = metadata
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    match auth {
        Some(a) if a.to_lowercase().starts_with("bearer ") => {
            let t = &a[7..a.len()];
            ModelTokenSecret::from_str(t.trim()).ok()
        }
        _ => None,
    }
}

pub async fn start_grpc_server(
    addr: SocketAddr,
    services: &Services,
    join_set: &mut JoinSet<Result<(), anyhow::Error>>,
) -> anyhow::Result<u16> {
    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();

    let listener = TcpListener::bind(addr).await?;
    let port = listener.local_addr()?.port();

    health_reporter
        .set_serving::<CloudAccountServiceServer<AccountGrpcApi>>()
        .await;
    health_reporter
        .set_serving::<CloudAuthServiceServer<AuthGrpcApi>>()
        .await;
    health_reporter
        .set_serving::<CloudLimitsServiceServer<LimitsGrpcApi>>()
        .await;
    health_reporter
        .set_serving::<CloudProjectServiceServer<ProjectGrpcApi>>()
        .await;
    health_reporter
        .set_serving::<CloudTokenServiceServer<TokenGrpcApi>>()
        .await;

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(golem_api_grpc::proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    join_set.spawn(
        Server::builder()
            .add_service(reflection_service)
            .add_service(health_service)
            .add_service(
                CloudAccountServiceServer::new(AccountGrpcApi {
                    auth_service: services.auth_service.clone(),
                    account_service: services.account_service.clone(),
                })
                .send_compressed(CompressionEncoding::Gzip)
                .accept_compressed(CompressionEncoding::Gzip),
            )
            .add_service(
                CloudAuthServiceServer::new(AuthGrpcApi {
                    auth_service: services.auth_service.clone(),
                })
                .send_compressed(CompressionEncoding::Gzip)
                .accept_compressed(CompressionEncoding::Gzip),
            )
            .add_service(
                CloudLimitsServiceServer::new(LimitsGrpcApi {
                    auth_service: services.auth_service.clone(),
                    plan_limit_service: services.plan_limit_service.clone(),
                })
                .send_compressed(CompressionEncoding::Gzip)
                .accept_compressed(CompressionEncoding::Gzip),
            )
            .add_service(
                CloudProjectServiceServer::new(ProjectGrpcApi {
                    auth_service: services.auth_service.clone(),
                    project_service: services.project_service.clone(),
                })
                .send_compressed(CompressionEncoding::Gzip)
                .accept_compressed(CompressionEncoding::Gzip),
            )
            .add_service(
                CloudTokenServiceServer::new(TokenGrpcApi {
                    auth_service: services.auth_service.clone(),
                    token_service: services.token_service.clone(),
                    login_system: services.login_system.clone(),
                })
                .send_compressed(CompressionEncoding::Gzip)
                .accept_compressed(CompressionEncoding::Gzip),
            )
            .serve_with_incoming(TcpListenerStream::new(listener))
            .map_err(anyhow::Error::from)
            .in_current_span(),
    );

    Ok(port)
}

#[cfg(test)]
mod tests {
    use test_r::test;

    use crate::grpcapi::get_authorisation_token;
    use golem_common::model::auth::TokenSecret as ModelTokenSecret;
    use tonic::metadata::MetadataMap;
    use uuid::Uuid;

    #[test]
    fn test_get_authorisation_token() {
        let mut m = MetadataMap::new();
        m.insert(
            "authorization",
            "Bearer 7E0BBC59-DB10-4A6F-B508-7673FE948315"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            get_authorisation_token(m),
            Some(ModelTokenSecret::new(
                Uuid::parse_str("7E0BBC59-DB10-4A6F-B508-7673FE948315").unwrap()
            ))
        );

        let mut m = MetadataMap::new();
        m.insert(
            "authorization",
            "bearer   7E0BBC59-DB10-4A6F-B508-7673FE948315 "
                .parse()
                .unwrap(),
        );
        assert_eq!(
            get_authorisation_token(m),
            Some(ModelTokenSecret::new(
                Uuid::parse_str("7E0BBC59-DB10-4A6F-B508-7673FE948315").unwrap()
            ))
        );

        let mut m = MetadataMap::new();
        m.insert("authorization", "Bearer token".parse().unwrap());
        assert_eq!(get_authorisation_token(m), None);

        let mut m = MetadataMap::new();
        m.insert("authorization", "Bearer ".parse().unwrap());
        assert_eq!(get_authorisation_token(m), None);

        let mut m = MetadataMap::new();
        m.insert("authorization", "Bearer".parse().unwrap());
        assert_eq!(get_authorisation_token(m), None);
    }
}
