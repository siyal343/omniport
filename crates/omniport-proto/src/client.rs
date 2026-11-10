use crate::codec::{read_message, write_message};
use crate::error::{ProtoError, Result};
use crate::types::{EndpointInfo, Request, Response, SerialConfigDto};
use tokio::net::TcpStream;

/// Control-plane client communicating with `omniportd`
pub struct RpcClient {
    stream: TcpStream,
}

impl RpcClient {
    /// Connects to omniportd at the given TCP address (e.g. "127.0.0.1:50051")
    pub async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self { stream })
    }

    /// Sends a low-level request and awaits the daemon's response
    pub async fn request(&mut self, req: Request) -> Result<Response> {
        write_message(&mut self.stream, &req).await?;
        read_message(&mut self.stream).await
    }

    /// Queries daemon health and uptime
    pub async fn health_check(&mut self) -> Result<(String, u64, u32, u32)> {
        match self.request(Request::HealthCheck).await? {
            Response::Health {
                status,
                uptime_secs,
                active_endpoints,
                active_routes,
            } => Ok((status, uptime_secs, active_endpoints, active_routes)),
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// Lists all active endpoints registered with omniportd
    pub async fn list_endpoints(&mut self) -> Result<Vec<EndpointInfo>> {
        match self.request(Request::ListEndpoints).await? {
            Response::Endpoints(eps) => Ok(eps),
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// Creates a virtual serial port pair (e.g. COM10 <-> COM11 or /tmp/ttyV0 <-> /tmp/ttyV1)
    pub async fn create_virtual_pair(
        &mut self,
        port_a: &str,
        port_b: &str,
        config: SerialConfigDto,
    ) -> Result<u32> {
        let req = Request::CreateVirtualPair {
            port_a: port_a.to_string(),
            port_b: port_b.to_string(),
            config,
        };
        match self.request(req).await? {
            Response::VirtualPairCreated {
                pair_id,
                success,
                message,
                ..
            } => {
                if success {
                    Ok(pair_id)
                } else {
                    Err(ProtoError::ServerError(message))
                }
            }
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// Adds a routing rule in the router graph
    pub async fn add_route(
        &mut self,
        source_id: u16,
        target_ids: Vec<u16>,
        bidirectional: bool,
    ) -> Result<u32> {
        let req = Request::AddRoute {
            source_id,
            target_ids,
            bidirectional,
        };
        match self.request(req).await? {
            Response::RouteAdded {
                route_id,
                success,
                message,
            } => {
                if success {
                    Ok(route_id)
                } else {
                    Err(ProtoError::ServerError(message))
                }
            }
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// Removes an existing route by its ID
    pub async fn remove_route(&mut self, route_id: u32) -> Result<()> {
        match self.request(Request::RemoveRoute { route_id }).await? {
            Response::Success { .. } => Ok(()),
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// Removes an endpoint by its ID
    pub async fn remove_endpoint(&mut self, id: u16) -> Result<()> {
        match self.request(Request::RemoveEndpoint { id }).await? {
            Response::Success { .. } => Ok(()),
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// Reads an ICD §2 encoded viewport batch from `omniportd`
    pub async fn read_viewport(&mut self, start_row: u64, row_count: u32) -> Result<Vec<u8>> {
        match self
            .request(Request::ReadViewport {
                start_row,
                row_count,
            })
            .await?
        {
            Response::ViewportBatch { bytes } => Ok(bytes),
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// Queries capture store packet and drop counters from `omniportd`
    pub async fn capture_status(&mut self) -> Result<(u64, u64)> {
        match self.request(Request::CaptureStatus).await? {
            Response::CaptureStatus {
                total_packets,
                dropped_frames,
            } => Ok((total_packets, dropped_frames)),
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// Asks the daemon to gracefully shut down
    pub async fn shutdown(&mut self) -> Result<()> {
        match self.request(Request::Shutdown).await? {
            Response::Success { .. } => Ok(()),
            Response::Error { message, .. } => Err(ProtoError::ServerError(message)),
            other => Err(ProtoError::UnexpectedResponse(format!("{:?}", other))),
        }
    }
}
