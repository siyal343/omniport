//! Tauri IPC commands for the Visual Topology editor.
//!
//! Every command is a thin adapter over [`crate::topology_store::TopologyControl`]
//! and returns the canonical `TopologyView` snapshot after the operation, so the
//! WebView always renders one source of truth.

use crate::topology_store::{
    AddRouteRequest, CreatePairRequest, DisconnectedTopology, TopologyControl, TopologyView,
};
use std::sync::Arc;
use tauri::State;

pub struct AppTopologyState {
    control: Arc<dyn TopologyControl>,
}

impl AppTopologyState {
    pub fn new(control: Arc<dyn TopologyControl>) -> Self {
        Self { control }
    }

    pub fn disconnected(reason: impl Into<String>) -> Self {
        Self::new(Arc::new(DisconnectedTopology::new(reason)))
    }

    pub async fn snapshot(&self) -> Result<TopologyView, String> {
        self.control.snapshot().await
    }

    pub async fn create_pair(&self, request: CreatePairRequest) -> Result<TopologyView, String> {
        self.control.create_pair(request).await
    }

    pub async fn remove_endpoint(&self, endpoint_id: u16) -> Result<TopologyView, String> {
        self.control.remove_endpoint(endpoint_id).await
    }

    pub async fn add_route(&self, request: AddRouteRequest) -> Result<TopologyView, String> {
        self.control.add_route(request).await
    }

    pub async fn remove_route(&self, route_id: u32) -> Result<TopologyView, String> {
        self.control.remove_route(route_id).await
    }
}

/// Returns the canonical endpoints and routes currently owned by the backend.
#[tauri::command]
pub async fn get_topology(state: State<'_, AppTopologyState>) -> Result<TopologyView, String> {
    state.snapshot().await
}

/// Provisions a virtual null-modem pair and its bidirectional route.
#[tauri::command]
pub async fn create_virtual_pair(
    state: State<'_, AppTopologyState>,
    request: CreatePairRequest,
) -> Result<TopologyView, String> {
    state.create_pair(request).await
}

/// Adds a directed routing rule (optionally with a fault injector).
#[tauri::command]
pub async fn add_route(
    state: State<'_, AppTopologyState>,
    request: AddRouteRequest,
) -> Result<TopologyView, String> {
    state.add_route(request).await
}

/// Removes a routing rule by id.
#[tauri::command]
pub async fn remove_route(
    state: State<'_, AppTopologyState>,
    route_id: u32,
) -> Result<TopologyView, String> {
    state.remove_route(route_id).await
}

/// Removes an endpoint by id (the backend prunes its routes atomically).
#[tauri::command]
pub async fn remove_endpoint(
    state: State<'_, AppTopologyState>,
    endpoint_id: u16,
) -> Result<TopologyView, String> {
    state.remove_endpoint(endpoint_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology_store::FixtureTopology;

    #[tokio::test]
    async fn test_app_topology_state_delegates_to_fixture() {
        let state = AppTopologyState::new(Arc::new(FixtureTopology::seeded().unwrap()));
        let view = state.snapshot().await.unwrap();
        assert_eq!(view.endpoints.len(), 3);
        assert_eq!(view.routes.len(), 2);

        let after_pair = state
            .create_pair(CreatePairRequest {
                port_a: "COM20".to_string(),
                port_b: "COM21".to_string(),
                baud_rate: 115_200,
                data_bits: 8,
                parity: 0,
                stop_bits: 1,
            })
            .await
            .unwrap();
        assert_eq!(after_pair.endpoints.len(), 5);
    }

    #[tokio::test]
    async fn test_app_topology_state_disconnected_surfaces_reason() {
        let state = AppTopologyState::disconnected("topology host unavailable");
        assert_eq!(
            state.snapshot().await.unwrap_err(),
            "topology host unavailable"
        );
    }
}
