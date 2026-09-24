//! Use cases. A service method is the unit of transaction: it validates a
//! command through the domain, opens the transaction, calls repositories,
//! publishes the resulting event and commits. HTTP, Matrix, MCP and LLM
//! tools all call the same method. Never imports a transport
//! (`tests/architecture.rs`).
