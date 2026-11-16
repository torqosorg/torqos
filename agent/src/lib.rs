//! TorqOS's agent, as a library its programs share (ADR-051, ADR-055): `torq-agent-service` (each person's
//! agent as a service: the bridge to their engine) and `torq-mcp` (TorqOS's tools for a coding tool,
//! ADR-051 7). One closed set of tools (`tools`), one way of cleaning what is shown (`clean`), and the
//! machine they run on (`machine`).

// No unsafe code and no dead code, ever (ADR-020); in a test build `deny`, as torqcli's (cli/src/main.rs).
#![forbid(unsafe_code)]
#![cfg_attr(not(test), forbid(dead_code))]
#![cfg_attr(test, deny(dead_code))]

// The cleaning is torq-wire's: the broker cleans an agent's text with the same one.
pub use torq_wire::clean;
pub mod link;
pub mod machine;
pub mod mcp;
pub mod tools;
