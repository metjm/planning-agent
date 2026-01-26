//! Tests for workflow supervisor.

use super::*;
use crate::domain::actor::create_actor_args;
use crate::planning_paths;
use tempfile::tempdir;

#[tokio::test]
async fn test_supervisor_spawn() {
    let dir = tempdir().expect("temp dir");
    let _guard = planning_paths::set_home_for_test(dir.path().to_path_buf());
    let session_id = uuid::Uuid::new_v4().to_string();

    let (args, _, _) = create_actor_args(&session_id).expect("create args failed");

    let (supervisor_ref, _handle) = WorkflowSupervisor::spawn(None, WorkflowSupervisor, ())
        .await
        .expect("supervisor spawn failed");

    supervisor_ref
        .send_message(SupervisorMsg::Spawn(args))
        .expect("send failed");

    // Give the actor time to spawn
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Supervisor should have spawned the actor
    // We can't easily verify this without more infrastructure, but at least it didn't panic
}
