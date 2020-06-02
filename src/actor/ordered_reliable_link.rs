//! An ordered reliable link based loosely on the "perfect link" described in
//! "Introduction to Reliable and Secure Distributed Programming" by Cachin,
//! Guerraoui, and Rodrigues (although that link was not ordered).

use crate::actor::*;
use serde::Deserialize;
use serde::Serialize;
use std::fmt::Debug;
use std::time::Duration;
use std::ops::Range;
use std::hash::Hash;
use std::collections::BTreeMap;

/// Wraps an actor with a "perfect link" providing the abstraction of a
/// lossless non-duplicating ordered network.
#[derive(Clone)]
pub struct ActorWrapper<A: Actor> {
    pub resend_interval: Range<Duration>,
    pub wrapped_actor: A,
}

/// Defines an interface for a register-like actor.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[derive(Serialize, Deserialize)]
pub enum MsgWrapper<Msg> {
    Deliver(Sequencer, Msg),
    Ack(Sequencer),
}

/// Perfect link sequencer.
pub type Sequencer = u64;

/// A wrapper state for model-checking a register-like actor.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StateWrapper<Msg, State> {
    // send side
    next_send_seq: Sequencer,
    msgs_pending_ack: BTreeMap<Sequencer, (Id, Msg)>,

    // receive (ack'ing) side
    last_delivered_seqs: BTreeMap<Id, Sequencer>,

    wrapped_state: State,
}

impl<A: Actor> Actor for ActorWrapper<A>
    where A::Msg: Hash
{
    type Msg = MsgWrapper<A::Msg>;
    type State = StateWrapper<A::Msg, A::State>;

    fn on_start(&self, id: Id, o: &mut Out<Self>) {
        o.set_timer(self.resend_interval.clone());

        let mut wrapped_out = self.wrapped_actor.on_start_out(id);
        let state = StateWrapper {
            next_send_seq: 1,
            msgs_pending_ack: Default::default(),
            last_delivered_seqs: Default::default(),
            wrapped_state: wrapped_out.state.take().expect(&format!(
                "on_start must assign state. id={:?}", id)),
        };
        process_output(wrapped_out, state, o);
    }

    fn on_msg(&self, id: Id, state: &Self::State, src: Id, msg: Self::Msg, o: &mut Out<Self>) {
        match msg {
            MsgWrapper::Deliver(seq, wrapped_msg) => {
                // Always ack the message to prevent re-sends, and early exit if already delivered.
                o.send(src, MsgWrapper::Ack(seq));
                if seq <= *state.last_delivered_seqs.get(&src).unwrap_or(&0) { return }

                // Process the message, and early exit if ignored.
                let wrapped_out = self.wrapped_actor.on_msg_out(id, &state.wrapped_state, src, wrapped_msg);
                if wrapped_out.is_no_op() { return }

                // Never delivered, and not ignored by actor, so update the sequencer and process the original output.
                let mut state = state.clone();
                state.last_delivered_seqs.insert(src, seq);
                process_output(wrapped_out, state, o);
            },
            MsgWrapper::Ack(seq) => {
                if !state.msgs_pending_ack.contains_key(&seq) { return }
                let mut state = state.clone();
                state.msgs_pending_ack.remove(&seq);
                o.set_state(state);
            },
        }
    }

    fn on_timeout(&self, _id: Id, state: &Self::State, o: &mut Out<Self>) {
        o.set_timer(self.resend_interval.clone());
        for (seq, (dst, msg)) in &state.msgs_pending_ack {
            o.send(*dst, MsgWrapper::Deliver(*seq, msg.clone()));
        }
    }
}

fn process_output<A: Actor>(wrapped_out: Out<A>, mut state: StateWrapper<A::Msg, A::State>, o: &mut Out<ActorWrapper<A>>)
where A::Msg: Hash
{
    if let Some(wrapped_state) = wrapped_out.state {
        state.wrapped_state = wrapped_state;
    }
    for command in wrapped_out.commands {
        match command {
            Command::CancelTimer => {
                todo!("CancelTimer is not supported at this time");
            },
            Command::SetTimer(_) => {
                todo!("SetTimer is not supported at this time");
            },
            Command::Send(dst, inner_msg) => {
                o.send(dst, MsgWrapper::Deliver(state.next_send_seq, inner_msg.clone()));
                state.msgs_pending_ack.insert(state.next_send_seq, (dst, inner_msg));
                state.next_send_seq += 1;
            },
        }
    }
    o.set_state(state);
}

#[cfg(test)]
mod test {
    use crate::{Property, Model};
    use crate::actor::{Actor, Id, Out};
    use crate::actor::ordered_reliable_link::{ActorWrapper, MsgWrapper};
    use crate::actor::system::{SystemModel, System, LossyNetwork, DuplicatingNetwork, SystemState};
    use std::time::Duration;
    use crate::actor::system::SystemAction;

    pub enum TestActor {
        Sender { receiver_id: Id },
        Receiver,
    }
    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    pub struct TestState {
        sent: Vec<(Id, TestMsg)>,
        received: Vec<(Id, TestMsg)>,
    }
    #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub struct TestMsg(u64);

    impl Actor for TestActor {
        type Msg = TestMsg;
        type State = TestState;

        fn on_start(&self, _id: Id, o: &mut Out<Self>) {
            let state = TestState {
                sent: Vec::new(),
                received: Vec::new(),
            };
            if let TestActor::Sender { receiver_id } = self {
                o.send(*receiver_id, TestMsg(42));
                o.send(*receiver_id, TestMsg(43));
            }
            o.set_state(state);
        }

        fn on_msg(&self, _id: Id, state: &Self::State, src: Id, msg: Self::Msg, o: &mut Out<Self>) {
            let mut state = state.clone();
            state.received.push((src, msg));
            o.set_state(state);
        }
    }

    struct TestSystem;
    impl System for TestSystem {
        type Actor = ActorWrapper<TestActor>;

        fn actors(&self) -> Vec<Self::Actor> {
            vec![
                ActorWrapper {
                    resend_interval: Duration::from_secs(1)..Duration::from_secs(2),
                    wrapped_actor: TestActor::Sender { receiver_id: Id::from(1) },
                },
                ActorWrapper {
                    resend_interval: Duration::from_secs(1)..Duration::from_secs(2),
                    wrapped_actor: TestActor::Receiver,
                },
            ]
        }

        fn lossy_network(&self) -> LossyNetwork {
            LossyNetwork::Yes
        }

        fn duplicating_network(&self) -> DuplicatingNetwork {
            DuplicatingNetwork::Yes
        }

        fn properties(&self) -> Vec<Property<SystemModel<Self>>> {
            vec![
                Property::<SystemModel<TestSystem>>::always("no redelivery", |_, state| {
                    let received = &state.actor_states[1].wrapped_state.received;
                    received.iter().filter(|(_, TestMsg(v))| *v == 42).count() < 2
                        && received.iter().filter(|(_, TestMsg(v))| *v == 43).count() < 2
                }),
                Property::<SystemModel<TestSystem>>::always("ordered", |_, state| {
                    state.actor_states[1].wrapped_state.received.iter()
                        .map(|(_, TestMsg(v))| *v)
                        .fold((true, 0), |(acc, last), next| (acc && last <= next, next))
                        .0
                }),
                // FIXME: convert to an eventually property once the liveness checker is complete
                Property::<SystemModel<TestSystem>>::sometimes("delivered", |_, state| {
                    state.actor_states[1].wrapped_state.received == vec![
                        (Id::from(0), TestMsg(42)),
                        (Id::from(0), TestMsg(43)),
                    ]
                }),
            ]
        }

        fn within_boundary(&self, state: &SystemState<Self::Actor>) -> bool {
            state.actor_states.iter().all(|s|
                s.wrapped_state.sent.len() < 4 && s.wrapped_state.received.len() < 4)
        }
    }

    #[test]
    fn messages_are_not_delivered_twice() {
        let mut checker = TestSystem.into_model().checker();
        checker.check(10_000).assert_no_counterexample("no redelivery");
    }

    #[test]
    fn messages_are_delivered_in_order() {
        let mut checker = TestSystem.into_model().checker();
        checker.check(10_000).assert_no_counterexample("ordered");
    }

    #[test]
    fn messages_are_eventually_delivered() {
        let mut checker = TestSystem.into_model().checker();
        assert_eq!(
            checker.check(10_000).assert_example("delivered").into_actions(),
            vec![
                SystemAction::Deliver { src: Id(0), dst: Id(1), msg: MsgWrapper::Deliver(1, TestMsg(42)) },
                SystemAction::Deliver { src: Id(0), dst: Id(1), msg: MsgWrapper::Deliver(2, TestMsg(43)) },
            ]);
    }
}