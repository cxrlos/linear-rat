//! Everything lrat needs from Linear goes through [`LinearClient`], so tests
//! and `--demo` swap the real client for [`FakeClient`].

pub mod model;

/// Queries, mutations and live events. It has none yet.
pub trait LinearClient {}

/// A client that never touches the network. It holds no data yet.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeClient;

impl LinearClient for FakeClient {}
