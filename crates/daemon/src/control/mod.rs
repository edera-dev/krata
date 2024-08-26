use std::pin::Pin;

use anyhow::Error;
use futures::Stream;
use list_network_reservations::ListNetworkReservationsRpc;
use tokio::sync::mpsc::Sender;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use krata::v1::control::{
    control_service_server::ControlService, CreateZoneReply, CreateZoneRequest, DestroyZoneReply,
    DestroyZoneRequest, ExecInsideZoneReply, ExecInsideZoneRequest, GetHostCpuTopologyReply,
    GetHostCpuTopologyRequest, GetHostStatusReply, GetHostStatusRequest, ListDevicesReply,
    ListDevicesRequest, ListZonesReply, ListZonesRequest, PullImageReply, PullImageRequest,
    ReadHypervisorConsoleReply, ReadHypervisorConsoleRequest, ReadZoneMetricsReply,
    ReadZoneMetricsRequest, ResolveZoneIdReply, ResolveZoneIdRequest, SnoopIdmReply,
    SnoopIdmRequest, UpdateZoneResourcesReply, UpdateZoneResourcesRequest, WatchEventsReply,
    WatchEventsRequest, ZoneConsoleReply, ZoneConsoleRequest,
};
use krata::v1::control::{
    GetZoneReply, GetZoneRequest, ListNetworkReservationsReply, ListNetworkReservationsRequest,
    SetHostPowerManagementPolicyReply, SetHostPowerManagementPolicyRequest,
};
use krataoci::packer::service::OciPackerService;
use kratart::Runtime;

use crate::control::attach_zone_console::AttachZoneConsoleRpc;
use crate::control::create_zone::CreateZoneRpc;
use crate::control::destroy_zone::DestroyZoneRpc;
use crate::control::exec_inside_zone::ExecInsideZoneRpc;
use crate::control::get_host_cpu_topology::GetHostCpuTopologyRpc;
use crate::control::get_host_status::GetHostStatusRpc;
use crate::control::get_zone::GetZoneRpc;
use crate::control::list_devices::ListDevicesRpc;
use crate::control::list_zones::ListZonesRpc;
use crate::control::pull_image::PullImageRpc;
use crate::control::read_hypervisor_console::ReadHypervisorConsoleRpc;
use crate::control::read_zone_metrics::ReadZoneMetricsRpc;
use crate::control::resolve_zone_id::ResolveZoneIdRpc;
use crate::control::set_host_power_management_policy::SetHostPowerManagementPolicyRpc;
use crate::control::snoop_idm::SnoopIdmRpc;
use crate::control::update_zone_resources::UpdateZoneResourcesRpc;
use crate::control::watch_events::WatchEventsRpc;
use crate::db::zone::ZoneStore;
use crate::network::assignment::NetworkAssignment;
use crate::{
    console::DaemonConsoleHandle, devices::DaemonDeviceManager, event::DaemonEventContext,
    idm::DaemonIdmHandle, zlt::ZoneLookupTable,
};

pub mod attach_zone_console;
pub mod create_zone;
pub mod destroy_zone;
pub mod exec_inside_zone;
pub mod get_host_cpu_topology;
pub mod get_host_status;
pub mod get_zone;
pub mod list_devices;
pub mod list_network_reservations;
pub mod list_zones;
pub mod pull_image;
pub mod read_hypervisor_console;
pub mod read_zone_metrics;
pub mod resolve_zone_id;
pub mod set_host_power_management_policy;
pub mod snoop_idm;
pub mod update_zone_resources;
pub mod watch_events;

pub struct ApiError {
    message: String,
}

impl From<Error> for ApiError {
    fn from(value: Error) -> Self {
        ApiError {
            message: value.to_string(),
        }
    }
}

impl From<ApiError> for Status {
    fn from(value: ApiError) -> Self {
        Status::unknown(value.message)
    }
}

#[derive(Clone)]
pub struct DaemonControlService {
    zlt: ZoneLookupTable,
    devices: DaemonDeviceManager,
    events: DaemonEventContext,
    console: DaemonConsoleHandle,
    idm: DaemonIdmHandle,
    zones: ZoneStore,
    network: NetworkAssignment,
    zone_reconciler_notify: Sender<Uuid>,
    packer: OciPackerService,
    runtime: Runtime,
}

impl DaemonControlService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zlt: ZoneLookupTable,
        devices: DaemonDeviceManager,
        events: DaemonEventContext,
        console: DaemonConsoleHandle,
        idm: DaemonIdmHandle,
        zones: ZoneStore,
        network: NetworkAssignment,
        zone_reconciler_notify: Sender<Uuid>,
        packer: OciPackerService,
        runtime: Runtime,
    ) -> Self {
        Self {
            zlt,
            devices,
            events,
            console,
            idm,
            zones,
            network,
            zone_reconciler_notify,
            packer,
            runtime,
        }
    }
}

#[tonic::async_trait]
impl ControlService for DaemonControlService {
    async fn get_host_status(
        &self,
        request: Request<GetHostStatusRequest>,
    ) -> Result<Response<GetHostStatusReply>, Status> {
        let request = request.into_inner();
        adapt(
            GetHostStatusRpc::new(self.network.clone(), self.zlt.clone())
                .process(request)
                .await,
        )
    }

    type SnoopIdmStream =
        Pin<Box<dyn Stream<Item = Result<SnoopIdmReply, Status>> + Send + 'static>>;

    async fn snoop_idm(
        &self,
        request: Request<SnoopIdmRequest>,
    ) -> Result<Response<Self::SnoopIdmStream>, Status> {
        let request = request.into_inner();
        adapt(
            SnoopIdmRpc::new(self.idm.clone(), self.zlt.clone())
                .process(request)
                .await,
        )
    }

    async fn get_host_cpu_topology(
        &self,
        request: Request<GetHostCpuTopologyRequest>,
    ) -> Result<Response<GetHostCpuTopologyReply>, Status> {
        let request = request.into_inner();
        adapt(
            GetHostCpuTopologyRpc::new(self.runtime.clone())
                .process(request)
                .await,
        )
    }

    async fn set_host_power_management_policy(
        &self,
        request: Request<SetHostPowerManagementPolicyRequest>,
    ) -> Result<Response<SetHostPowerManagementPolicyReply>, Status> {
        let request = request.into_inner();
        adapt(
            SetHostPowerManagementPolicyRpc::new(self.runtime.clone())
                .process(request)
                .await,
        )
    }

    async fn list_devices(
        &self,
        request: Request<ListDevicesRequest>,
    ) -> Result<Response<ListDevicesReply>, Status> {
        let request = request.into_inner();
        adapt(
            ListDevicesRpc::new(self.devices.clone())
                .process(request)
                .await,
        )
    }

    async fn list_network_reservations(
        &self,
        request: Request<ListNetworkReservationsRequest>,
    ) -> Result<Response<ListNetworkReservationsReply>, Status> {
        let request = request.into_inner();
        adapt(
            ListNetworkReservationsRpc::new(self.network.clone())
                .process(request)
                .await,
        )
    }

    type PullImageStream =
        Pin<Box<dyn Stream<Item = Result<PullImageReply, Status>> + Send + 'static>>;

    async fn pull_image(
        &self,
        request: Request<PullImageRequest>,
    ) -> Result<Response<Self::PullImageStream>, Status> {
        let request = request.into_inner();
        adapt(
            PullImageRpc::new(self.packer.clone())
                .process(request)
                .await,
        )
    }

    async fn create_zone(
        &self,
        request: Request<CreateZoneRequest>,
    ) -> Result<Response<CreateZoneReply>, Status> {
        let request = request.into_inner();
        adapt(
            CreateZoneRpc::new(
                self.zones.clone(),
                self.zlt.clone(),
                self.zone_reconciler_notify.clone(),
            )
            .process(request)
            .await,
        )
    }

    async fn destroy_zone(
        &self,
        request: Request<DestroyZoneRequest>,
    ) -> Result<Response<DestroyZoneReply>, Status> {
        let request = request.into_inner();
        adapt(
            DestroyZoneRpc::new(self.zones.clone(), self.zone_reconciler_notify.clone())
                .process(request)
                .await,
        )
    }

    async fn resolve_zone_id(
        &self,
        request: Request<ResolveZoneIdRequest>,
    ) -> Result<Response<ResolveZoneIdReply>, Status> {
        let request = request.into_inner();
        adapt(
            ResolveZoneIdRpc::new(self.zones.clone())
                .process(request)
                .await,
        )
    }

    async fn get_zone(
        &self,
        request: Request<GetZoneRequest>,
    ) -> Result<Response<GetZoneReply>, Status> {
        let request = request.into_inner();
        adapt(GetZoneRpc::new(self.zones.clone()).process(request).await)
    }

    async fn update_zone_resources(
        &self,
        request: Request<UpdateZoneResourcesRequest>,
    ) -> Result<Response<UpdateZoneResourcesReply>, Status> {
        let request = request.into_inner();
        adapt(
            UpdateZoneResourcesRpc::new(self.runtime.clone(), self.zones.clone())
                .process(request)
                .await,
        )
    }

    async fn list_zones(
        &self,
        request: Request<ListZonesRequest>,
    ) -> Result<Response<ListZonesReply>, Status> {
        let request = request.into_inner();
        adapt(ListZonesRpc::new(self.zones.clone()).process(request).await)
    }

    type AttachZoneConsoleStream =
        Pin<Box<dyn Stream<Item = Result<ZoneConsoleReply, Status>> + Send + 'static>>;

    async fn attach_zone_console(
        &self,
        request: Request<Streaming<ZoneConsoleRequest>>,
    ) -> Result<Response<Self::AttachZoneConsoleStream>, Status> {
        let input = request.into_inner();
        adapt(
            AttachZoneConsoleRpc::new(self.console.clone())
                .process(input)
                .await,
        )
    }

    type ExecInsideZoneStream =
        Pin<Box<dyn Stream<Item = Result<ExecInsideZoneReply, Status>> + Send + 'static>>;

    async fn exec_inside_zone(
        &self,
        request: Request<Streaming<ExecInsideZoneRequest>>,
    ) -> Result<Response<Self::ExecInsideZoneStream>, Status> {
        let input = request.into_inner();
        adapt(
            ExecInsideZoneRpc::new(self.idm.clone())
                .process(input)
                .await,
        )
    }

    async fn read_zone_metrics(
        &self,
        request: Request<ReadZoneMetricsRequest>,
    ) -> Result<Response<ReadZoneMetricsReply>, Status> {
        let request = request.into_inner();
        adapt(
            ReadZoneMetricsRpc::new(self.idm.clone())
                .process(request)
                .await,
        )
    }

    type WatchEventsStream =
        Pin<Box<dyn Stream<Item = Result<WatchEventsReply, Status>> + Send + 'static>>;

    async fn watch_events(
        &self,
        request: Request<WatchEventsRequest>,
    ) -> Result<Response<Self::WatchEventsStream>, Status> {
        let request = request.into_inner();
        adapt(
            WatchEventsRpc::new(self.events.clone())
                .process(request)
                .await,
        )
    }

    async fn read_hypervisor_console(
        &self,
        request: Request<ReadHypervisorConsoleRequest>,
    ) -> Result<Response<ReadHypervisorConsoleReply>, Status> {
        let request = request.into_inner();
        adapt(
            ReadHypervisorConsoleRpc::new(self.runtime.clone())
                .process(request)
                .await,
        )
    }
}

fn adapt<T>(result: anyhow::Result<T>) -> Result<Response<T>, Status> {
    result
        .map(Response::new)
        .map_err(|error| Status::unknown(error.to_string()))
}
