//! Workflow supervisor for fault-tolerant actor management.
//!
//! The supervisor monitors workflow actors and automatically restarts
//! them if they fail or terminate unexpectedly.

use crate::domain::actor::{WorkflowActor, WorkflowActorArgs};
use async_trait::async_trait;
use ractor::{Actor, ActorProcessingErr, ActorRef, SupervisionEvent};

/// Messages for the workflow supervisor.
pub enum SupervisorMsg {
    /// Spawn a new workflow actor.
    Spawn(WorkflowActorArgs),
}

/// The workflow supervisor actor.
pub struct WorkflowSupervisor;

#[async_trait]
impl Actor for WorkflowSupervisor {
    type Msg = SupervisorMsg;
    type State = Option<WorkflowActorArgs>;
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _args: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(None)
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        msg: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match msg {
            SupervisorMsg::Spawn(args) => {
                *state = Some(args.clone());
                let _ = WorkflowActor::spawn_linked(None, WorkflowActor, args, myself.get_cell())
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        evt: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if matches!(
            evt,
            SupervisionEvent::ActorFailed(_, _) | SupervisionEvent::ActorTerminated(_, _, _)
        ) {
            if let Some(args) = state.clone() {
                let _ = WorkflowActor::spawn_linked(None, WorkflowActor, args, myself.get_cell())
                    .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/supervisor_tests.rs"]
mod tests;
