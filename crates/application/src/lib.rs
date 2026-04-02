pub mod assistant_mode_service;
pub mod commands;
pub mod error;
pub mod mode_coordinator_service;
pub mod multi_agent_orchestrator_service;
pub mod queries;
pub mod remote_session_service;
pub mod spec_contracts;
pub mod teammem_kairos_service;
pub mod ultraplan_service;

#[cfg(test)]
mod bridge_mode_acceptance_tests;
#[cfg(test)]
mod coordinator_mode_acceptance_tests;
#[cfg(test)]
mod kairos_acceptance_tests;
#[cfg(test)]
mod multi_agent_orchestration_acceptance_tests;
#[cfg(test)]
mod multi_agent_orchestrator_tests;
#[cfg(test)]
mod remote_session_service_tests;
