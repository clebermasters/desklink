use anyhow::{Result, bail, ensure};
use desklink_protocol::{Edge, Geometry, Input, Message, Stamp};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
type Peer = [u8; 32];

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Send(Peer, Message),
    Capture,
    ReleaseCapture,
    Warp(i32, i32),
    Inject(Input),
    Status(&'static str),
}

#[derive(Debug)]
enum State {
    Local,
    Sending {
        peer: Peer,
        session: Stamp,
        edge: Edge,
        ready: bool,
        queued: Vec<Input>,
        since: Instant,
    },
    Receiving {
        peer: Peer,
        session: Stamp,
        edge: Edge,
        x: i32,
        y: i32,
    },
}

pub struct Engine {
    own: Peer,
    sequence: u64,
    geometry: Geometry,
    routes: Vec<(Edge, Peer)>,
    connected: BTreeSet<Peer>,
    state: State,
    keys: BTreeSet<u16>,
    injected_keys: BTreeSet<u16>,
    injected_buttons: BTreeSet<u8>,
    cooldown: Instant,
}

impl Engine {
    pub fn new(own: Peer, geometry: Geometry, routes: Vec<(Edge, Peer)>) -> Self {
        Self {
            own,
            sequence: 0,
            geometry,
            routes,
            connected: BTreeSet::new(),
            state: State::Local,
            keys: BTreeSet::new(),
            injected_keys: BTreeSet::new(),
            injected_buttons: BTreeSet::new(),
            cooldown: Instant::now(),
        }
    }
    pub fn connected(&mut self, peer: Peer) {
        self.connected.insert(peer);
    }
    pub fn disconnected(&mut self, peer: Peer) -> Vec<Effect> {
        self.connected.remove(&peer);
        if self.active_peer() == Some(peer) {
            self.abort()
        } else {
            Vec::new()
        }
    }
    pub fn active_peer(&self) -> Option<Peer> {
        match self.state {
            State::Local => None,
            State::Sending { peer, .. } | State::Receiving { peer, .. } => Some(peer),
        }
    }
    pub fn pointer(&mut self, x: i32, y: i32, dx: i32, dy: i32) -> Vec<Effect> {
        if !matches!(self.state, State::Local) || Instant::now() < self.cooldown {
            return Vec::new();
        }
        let Some((edge, position)) = self.geometry.edge_at(x, y, dx, dy) else {
            return Vec::new();
        };
        let Some(peer) = self
            .routes
            .iter()
            .find(|(e, p)| *e == edge && self.connected.contains(p))
            .map(|(_, p)| *p)
        else {
            return Vec::new();
        };
        self.sequence = self.sequence.wrapping_add(1).max(1);
        let session = Stamp {
            counter: self.sequence,
            origin: self.own,
        };
        self.state = State::Sending {
            peer,
            session,
            edge,
            ready: false,
            queued: Vec::new(),
            since: Instant::now(),
        };
        vec![
            Effect::Capture,
            Effect::Send(
                peer,
                Message::Enter {
                    session,
                    edge: edge.opposite(),
                    position,
                },
            ),
            Effect::Status("switching to peer"),
        ]
    }
    pub fn local_input(&mut self, event: Input) -> Result<Vec<Effect>> {
        if let Input::Key { code, down } = event {
            if down {
                self.keys.insert(code);
            } else {
                self.keys.remove(&code);
            }
        }
        if self.keys.contains(&1)
            && (self.keys.contains(&29) || self.keys.contains(&97))
            && (self.keys.contains(&56) || self.keys.contains(&100))
            && (self.keys.contains(&42) || self.keys.contains(&54))
        {
            return Ok(self.abort());
        }
        if let State::Sending {
            peer,
            session,
            ready,
            queued,
            ..
        } = &mut self.state
        {
            ensure!(event.valid(), "invalid local input");
            if *ready {
                return Ok(vec![Effect::Send(
                    *peer,
                    Message::Input {
                        session: *session,
                        event,
                    },
                )]);
            }
            ensure!(
                queued.len() < 256,
                "peer did not acknowledge capture promptly"
            );
            queued.push(event);
        }
        Ok(Vec::new())
    }
    pub fn remote(&mut self, peer: Peer, message: Message) -> Result<Vec<Effect>> {
        ensure!(
            self.connected.contains(&peer),
            "message from disconnected peer"
        );
        match message {
            Message::Enter {
                session,
                edge,
                position,
            } => {
                ensure!(
                    session.origin == peer
                        && session.counter > 0
                        && position.is_finite()
                        && (0.0..=1.0).contains(&position),
                    "invalid enter request"
                );
                let mut effects = Vec::new();
                // Deterministic arbitration if both users cross a boundary simultaneously.
                if matches!(self.state,State::Sending {peer:p,..}if p==peer) && self.own > peer {
                    effects.extend(self.abort());
                }
                if !matches!(self.state, State::Local) {
                    effects.push(Effect::Send(peer, Message::Release(session)));
                    return Ok(effects);
                }
                let (x, y) = self.geometry.entry(edge, position);
                self.state = State::Receiving {
                    peer,
                    session,
                    edge,
                    x,
                    y,
                };
                effects.extend([
                    Effect::Warp(x, y),
                    Effect::Send(peer, Message::Entered(session)),
                    Effect::Status("receiving input"),
                ]);
                Ok(effects)
            }
            Message::Entered(ack) => {
                if let State::Sending {
                    peer: p,
                    session,
                    ready,
                    queued,
                    ..
                } = &mut self.state
                    && *p == peer
                    && *session == ack
                {
                    *ready = true;
                    return Ok(queued
                        .drain(..)
                        .map(|event| {
                            Effect::Send(
                                peer,
                                Message::Input {
                                    session: ack,
                                    event,
                                },
                            )
                        })
                        .collect());
                }
                Ok(Vec::new())
            }
            Message::Input {
                session: token,
                event,
            } => {
                ensure!(event.valid(), "invalid input message");
                let State::Receiving {
                    peer: p,
                    session,
                    edge,
                    x,
                    y,
                } = &mut self.state
                else {
                    return Ok(Vec::new());
                };
                if *p != peer || *session != token {
                    return Ok(Vec::new());
                }
                match event {
                    Input::Motion { dx, dy } => {
                        let (nx, ny) = self
                            .geometry
                            .clamp(x.saturating_add(dx), y.saturating_add(dy));
                        if let Some((exit, position)) = self.geometry.edge_at(nx, ny, dx, dy)
                            && exit == *edge
                        {
                            let mut effects = self.reset();
                            effects.push(Effect::Send(
                                peer,
                                Message::Return {
                                    session: token,
                                    position,
                                },
                            ));
                            return Ok(effects);
                        }
                        *x = nx;
                        *y = ny;
                        Ok(vec![Effect::Warp(nx, ny)])
                    }
                    Input::Key { code, down } => {
                        if down {
                            self.injected_keys.insert(code);
                        } else {
                            self.injected_keys.remove(&code);
                        }
                        Ok(vec![Effect::Inject(event)])
                    }
                    Input::Button { button, down } => {
                        if down {
                            self.injected_buttons.insert(button);
                        } else {
                            self.injected_buttons.remove(&button);
                        }
                        Ok(vec![Effect::Inject(event)])
                    }
                    Input::Scroll { .. } => Ok(vec![Effect::Inject(event)]),
                }
            }
            Message::Return {
                session: token,
                position,
            } => {
                ensure!(
                    position.is_finite() && (0.0..=1.0).contains(&position),
                    "invalid return position"
                );
                if let State::Sending {
                    peer: p,
                    session,
                    edge,
                    ..
                } = self.state
                    && p == peer
                    && session == token
                {
                    let (x, y) = self.geometry.entry(edge, position);
                    let mut effects = self.reset();
                    effects.push(Effect::Warp(x, y));
                    return Ok(effects);
                }
                Ok(Vec::new())
            }
            Message::Release(token) => {
                let matching = match self.state {
                    State::Sending {
                        peer: p, session, ..
                    }
                    | State::Receiving {
                        peer: p, session, ..
                    } => p == peer && session == token,
                    State::Local => false,
                };
                Ok(if matching { self.reset() } else { Vec::new() })
            }
            Message::Geometry(g) => {
                ensure!(g.valid(), "invalid peer geometry");
                Ok(Vec::new())
            }
            _ => bail!("unexpected control message"),
        }
    }
    pub fn tick(&mut self) -> Vec<Effect> {
        if matches!(&self.state,State::Sending {ready:false,since,..}if since.elapsed()>Duration::from_secs(3))
        {
            self.abort()
        } else {
            Vec::new()
        }
    }
    pub fn geometry_changed(&mut self, g: Geometry) -> Vec<Effect> {
        self.geometry = g;
        let mut effects = self.abort();
        effects.extend(
            self.connected
                .iter()
                .map(|p| Effect::Send(*p, Message::Geometry(g))),
        );
        effects
    }
    pub fn abort(&mut self) -> Vec<Effect> {
        let active = match self.state {
            State::Local => None,
            State::Sending { peer, session, .. } | State::Receiving { peer, session, .. } => {
                Some((peer, session))
            }
        };
        let mut effects = self.reset();
        if let Some((peer, session)) = active {
            effects.push(Effect::Send(peer, Message::Release(session)));
        }
        effects
    }
    fn reset(&mut self) -> Vec<Effect> {
        self.state = State::Local;
        self.cooldown = Instant::now() + Duration::from_millis(250);
        self.keys.clear();
        let mut effects = vec![Effect::ReleaseCapture, Effect::Status("local control")];
        effects.extend(
            std::mem::take(&mut self.injected_keys)
                .into_iter()
                .map(|code| Effect::Inject(Input::Key { code, down: false })),
        );
        effects.extend(
            std::mem::take(&mut self.injected_buttons)
                .into_iter()
                .map(|button| {
                    Effect::Inject(Input::Button {
                        button,
                        down: false,
                    })
                }),
        );
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn g() -> Geometry {
        Geometry {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        }
    }
    fn engine() -> Engine {
        let mut e = Engine::new([1; 32], g(), vec![(Edge::Right, [2; 32])]);
        e.connected([2; 32]);
        e
    }
    fn token() -> Stamp {
        Stamp {
            origin: [2; 32],
            counter: 1,
        }
    }
    #[test]
    fn input_is_not_sent_before_enter_ack() {
        let mut e = engine();
        let actions = e.pointer(799, 300, 4, 0);
        let session = match actions[1] {
            Effect::Send(_, Message::Enter { session, .. }) => session,
            _ => panic!(),
        };
        let event = Input::Key {
            code: 30,
            down: true,
        };
        assert!(e.local_input(event).unwrap().is_empty());
        assert_eq!(
            e.remote([2; 32], Message::Entered(session)).unwrap(),
            vec![Effect::Send([2; 32], Message::Input { session, event })]
        );
    }
    #[test]
    fn disconnect_releases_injected_modifiers_and_buttons() {
        let mut e = engine();
        e.remote(
            [2; 32],
            Message::Enter {
                session: token(),
                edge: Edge::Left,
                position: 0.5,
            },
        )
        .unwrap();
        for event in [
            Input::Key {
                code: 29,
                down: true,
            },
            Input::Button {
                button: 1,
                down: true,
            },
        ] {
            e.remote(
                [2; 32],
                Message::Input {
                    session: token(),
                    event,
                },
            )
            .unwrap();
        }
        let effects = e.disconnected([2; 32]);
        assert!(effects.contains(&Effect::Inject(Input::Key {
            code: 29,
            down: false
        })));
        assert!(effects.contains(&Effect::Inject(Input::Button {
            button: 1,
            down: false
        })));
        assert_eq!(e.active_peer(), None);
    }
    #[test]
    fn returns_across_entry_edge_and_ignores_stale_input() {
        let mut e = engine();
        e.remote(
            [2; 32],
            Message::Enter {
                session: token(),
                edge: Edge::Left,
                position: 0.5,
            },
        )
        .unwrap();
        let effects = e
            .remote(
                [2; 32],
                Message::Input {
                    session: token(),
                    event: Input::Motion { dx: -10, dy: 0 },
                },
            )
            .unwrap();
        assert!(
            effects
                .iter()
                .any(|x| matches!(x, Effect::Send(_, Message::Return { .. })))
        );
        assert!(
            e.remote(
                [2; 32],
                Message::Input {
                    session: token(),
                    event: Input::Key {
                        code: 30,
                        down: true
                    }
                }
            )
            .unwrap()
            .is_empty()
        );
    }
    #[test]
    fn forged_session_and_nonfinite_coordinates_fail() {
        let mut e = engine();
        assert!(
            e.remote(
                [2; 32],
                Message::Enter {
                    session: Stamp {
                        origin: [3; 32],
                        counter: 1
                    },
                    edge: Edge::Left,
                    position: 0.5
                }
            )
            .is_err()
        );
        assert!(
            e.remote(
                [2; 32],
                Message::Enter {
                    session: token(),
                    edge: Edge::Left,
                    position: f64::NAN
                }
            )
            .is_err()
        );
    }
    #[test]
    fn emergency_chord_restores_local_control() {
        let mut e = engine();
        e.pointer(799, 200, 1, 0);
        for code in [29, 56, 42] {
            e.local_input(Input::Key { code, down: true }).unwrap();
        }
        let effects = e
            .local_input(Input::Key {
                code: 1,
                down: true,
            })
            .unwrap();
        assert!(effects.contains(&Effect::ReleaseCapture));
        assert_eq!(e.active_peer(), None);
    }
    #[test]
    fn simultaneous_handoffs_choose_one_controller_and_ignore_the_losing_release() {
        let mut a = engine();
        let mut b = Engine::new([2; 32], g(), vec![(Edge::Left, [1; 32])]);
        b.connected([1; 32]);
        let ae = a.pointer(799, 300, 1, 0);
        let be = b.pointer(0, 300, -1, 0);
        let request = |effects: Vec<Effect>| {
            effects
                .into_iter()
                .find_map(|e| {
                    if let Effect::Send(_, m @ Message::Enter { .. }) = e {
                        Some(m)
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        let aa = request(ae);
        let bb = request(be);
        let reject = a.remote([2; 32], bb).unwrap();
        let accepted = b.remote([1; 32], aa).unwrap();
        assert!(accepted.contains(&Effect::ReleaseCapture));
        for effect in reject {
            if let Effect::Send(_, message) = effect {
                b.remote([1; 32], message).unwrap();
            }
        }
        for effect in accepted {
            if let Effect::Send(_, message) = effect {
                a.remote([2; 32], message).unwrap();
            }
        }
        assert!(matches!(a.state, State::Sending { ready: true, .. }));
        assert!(matches!(b.state, State::Receiving { .. }));
    }
}
