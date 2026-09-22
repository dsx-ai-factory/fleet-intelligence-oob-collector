// The large serde_json::json! golden inventory test nests processors under
// systems deeply enough to exceed the default macro recursion limit.
#![cfg_attr(test, recursion_limit = "256")]

pub mod collector;
pub mod config;
pub mod mapper;
pub mod models;
pub mod sink;
pub mod source;
