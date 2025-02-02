use super::{
  serialize::Serialize,
  message::Message,
  request::Request,
  response::Response,
};

pub trait Target {
  type Inbound: Message;
  type Outbound: Message + Serialize;
}

pub struct AsClient;

impl Target for AsClient {
  type Inbound = Response;
  type Outbound = Request;
}

pub struct AsServer;

impl Target for AsServer {
  type Inbound = Request;
  type Outbound = Response;
}