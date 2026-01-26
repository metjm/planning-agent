use super::*;
use serial_test::serial;

#[test]
#[serial]
fn test_host_port_default() {
    // Clear env var if set
    std::env::remove_var("PLANNING_AGENT_HOST_PORT");
    assert_eq!(host_port(), Some(DEFAULT_HOST_PORT));
}

#[test]
#[serial]
fn test_host_port_custom() {
    std::env::set_var("PLANNING_AGENT_HOST_PORT", "12345");
    assert_eq!(host_port(), Some(12345));
    std::env::remove_var("PLANNING_AGENT_HOST_PORT");
}

#[test]
#[serial]
fn test_host_port_disabled() {
    std::env::set_var("PLANNING_AGENT_HOST_PORT", "0");
    assert_eq!(host_port(), None);
    std::env::remove_var("PLANNING_AGENT_HOST_PORT");
}
