use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestInfo {
    pub id: String,
    pub image: String,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchRequest {
    pub image: String,
    pub vcpus: u32,
    pub mem: u64,
    pub env: Option<Vec<String>>,
    pub run: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchResponse {
    pub guest: GuestInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    pub guests: Vec<GuestInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyRequest {
    pub guest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyResponse {
    pub guest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleStreamRequest {
    pub guest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleStreamResponse {
    pub stream: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleStreamUpdate {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Launch(LaunchRequest),
    Destroy(DestroyRequest),
    List(ListRequest),
    ConsoleStream(ConsoleStreamRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Error(ErrorResponse),
    Launch(LaunchResponse),
    Destroy(DestroyResponse),
    List(ListResponse),
    ConsoleStream(ConsoleStreamResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBox {
    pub id: u64,
    pub request: Request,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseBox {
    pub id: u64,
    pub response: Response,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StreamStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamUpdate {
    ConsoleStream(ConsoleStreamUpdate),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamUpdated {
    pub id: u64,
    pub update: Option<StreamUpdate>,
    pub status: StreamStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Request(RequestBox),
    Response(ResponseBox),
    StreamUpdated(StreamUpdated),
}
