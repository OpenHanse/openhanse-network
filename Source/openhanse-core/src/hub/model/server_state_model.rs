use crate::{
    hub::{
        model::{
            api_error_model::ApiErrorModel,
            peer_model::PeerPresenceModel,
            relay_session_model::RelaySessionModel,
        },
        util::time_util::TimeUtil,
    },
    model::{
        connect_model::{
            ConnectDecisionModel, ConnectRequestModel, DirectConnectionInfoModel,
            RelayConnectionInfoModel,
        },
        peer_model::{
            DirectReachabilityModeModel, NatBehaviorModel, PeerReachabilityModel,
            PeerRecordModel, ReachabilityAddressModel, ReachabilityConfidenceModel,
            ReachabilityScopeModel, ReachabilitySourceModel, RegisterPeerRequestModel,
            TransportProtocolEnum,
        },
        relay_model::{
            RelayAttachResponseModel, RelayMessageEnvelopeModel, RelaySendRequestModel,
            RelaySendResponseModel,
        },
    },
};
use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct ServerStateModel {
    pub peers: HashMap<String, PeerPresenceModel>,
    pub relay_sessions: HashMap<Uuid, RelaySessionModel>,
    pub presence_lease: Duration,
    pub relay_session_timeout: Duration,
}

impl ServerStateModel {
    pub fn new(presence_lease: Duration, relay_session_timeout: Duration) -> Self {
        Self {
            peers: HashMap::new(),
            relay_sessions: HashMap::new(),
            presence_lease,
            relay_session_timeout,
        }
    }

    pub fn register_peer(
        &mut self,
        request: RegisterPeerRequestModel,
        observed_remote_addr: Option<std::net::SocketAddr>,
    ) -> PeerRecordModel {
        let now = Instant::now();
        let reachability = apply_observed_reachability(request.reachability, observed_remote_addr);
        let peer = PeerPresenceModel {
            peer_id: request.peer_id,
            device_key: request.device_key,
            display_name: request.display_name,
            reachability,
            registered_at: now,
            expires_at: now + self.presence_lease,
        };
        self.peers.insert(peer.peer_id.clone(), peer.clone());
        peer.to_record_model()
    }

    pub fn heartbeat(&mut self, peer_id: &str) -> Option<PeerRecordModel> {
        let lease = self.presence_lease;
        let peer = self.peers.get_mut(peer_id)?;
        peer.expires_at = Instant::now() + lease;
        Some(peer.to_record_model())
    }

    pub fn lookup_peer(&mut self, peer_id: &str) -> Option<PeerRecordModel> {
        self.prune_expired();
        self.peers
            .get(peer_id)
            .map(PeerPresenceModel::to_record_model)
    }

    pub fn connect(
        &mut self,
        request: ConnectRequestModel,
    ) -> Result<ConnectDecisionModel, ApiErrorModel> {
        self.prune_expired();

        let source = self
            .peers
            .get(&request.source_peer_id)
            .cloned()
            .ok_or_else(|| {
                ApiErrorModel::not_found(format!(
                    "source peer '{}' is offline",
                    request.source_peer_id
                ))
            })?;
        let target = self
            .peers
            .get(&request.target_peer_id)
            .cloned()
            .ok_or_else(|| {
                ApiErrorModel::not_found(format!(
                    "target peer '{}' is offline",
                    request.target_peer_id
                ))
            })?;

        if request.source_peer_id == request.target_peer_id {
            return Err(ApiErrorModel::bad_request(
                "source and target peer must differ",
            ));
        }

        if request.prefer_direct {
            if let Some((candidates, decision_reason)) =
                select_direct_candidates(&source, &target)
            {
            return Ok(ConnectDecisionModel::direct(DirectConnectionInfoModel {
                peer_id: target.peer_id,
                device_key: target.device_key,
                display_name: target.display_name,
                    reachability_candidates: candidates,
                    message_endpoint: target.reachability.message_endpoint.clone(),
                    decision_reason,
            }));
            }
        }

        let relay_session_id = Uuid::new_v4();
        let relay_session = RelaySessionModel {
            source_peer_id: request.source_peer_id,
            target_peer_id: request.target_peer_id,
            expires_at: Instant::now() + self.relay_session_timeout,
            source_attached: false,
            target_attached: false,
            pending_for_source: VecDeque::new(),
            pending_for_target: VecDeque::new(),
        };
        self.relay_sessions
            .insert(relay_session_id, relay_session.clone());

        Ok(ConnectDecisionModel::relay(RelayConnectionInfoModel {
            relay_session_id,
            source_peer_id: relay_session.source_peer_id,
            target_peer_id: relay_session.target_peer_id,
            expires_at_unix_ms: TimeUtil::unix_time_ms_from_now(self.relay_session_timeout),
            decision_reason: relay_decision_reason(&source, &target, request.prefer_direct),
        }))
    }

    pub fn attach_relay_peer(
        &mut self,
        relay_session_id: Uuid,
        peer_id: &str,
    ) -> Result<RelayAttachResponseModel, ApiErrorModel> {
        self.prune_expired();

        let session = self
            .relay_sessions
            .get_mut(&relay_session_id)
            .ok_or_else(|| ApiErrorModel::not_found(format!("relay session '{relay_session_id}' not found")))?;

        let counterpart_peer_id = if peer_id == session.source_peer_id {
            session.source_attached = true;
            session.target_peer_id.clone()
        } else if peer_id == session.target_peer_id {
            session.target_attached = true;
            session.source_peer_id.clone()
        } else {
            return Err(ApiErrorModel::bad_request(format!(
                "peer '{peer_id}' is not part of relay session '{relay_session_id}'"
            )));
        };

        session.expires_at = Instant::now() + self.relay_session_timeout;

        Ok(RelayAttachResponseModel {
            accepted: true,
            relay_session_id,
            peer_id: peer_id.to_string(),
            counterpart_peer_id,
            expires_at_unix_ms: TimeUtil::unix_time_ms_from_now(self.relay_session_timeout),
        })
    }

    pub fn relay_send(
        &mut self,
        request: RelaySendRequestModel,
    ) -> Result<RelaySendResponseModel, ApiErrorModel> {
        self.prune_expired();

        let session = self
            .relay_sessions
            .get_mut(&request.relay_session_id)
            .ok_or_else(|| {
                ApiErrorModel::not_found(format!(
                    "relay session '{}' not found",
                    request.relay_session_id
                ))
            })?;

        let (recipient_peer_id, queue) = if request.peer_id == session.source_peer_id {
            if request.payload.from_peer_id != session.source_peer_id
                || request.payload.to_peer_id != session.target_peer_id
            {
                return Err(ApiErrorModel::bad_request(
                    "relay payload does not match source/target session peers",
                ));
            }
            (session.target_peer_id.clone(), &mut session.pending_for_target)
        } else if request.peer_id == session.target_peer_id {
            if request.payload.from_peer_id != session.target_peer_id
                || request.payload.to_peer_id != session.source_peer_id
            {
                return Err(ApiErrorModel::bad_request(
                    "relay payload does not match source/target session peers",
                ));
            }
            (session.source_peer_id.clone(), &mut session.pending_for_source)
        } else {
            return Err(ApiErrorModel::bad_request(format!(
                "peer '{}' is not part of relay session '{}'",
                request.peer_id, request.relay_session_id
            )));
        };

        queue.push_back(RelayMessageEnvelopeModel {
            relay_session_id: request.relay_session_id,
            source_peer_id: request.payload.from_peer_id.clone(),
            target_peer_id: request.payload.to_peer_id.clone(),
            queued_at_unix_ms: TimeUtil::unix_time_ms_from_now(Duration::ZERO),
            payload: request.payload,
        });
        session.expires_at = Instant::now() + self.relay_session_timeout;

        Ok(RelaySendResponseModel {
            accepted: true,
            relay_session_id: request.relay_session_id,
            recipient_peer_id,
            queued_messages: queue.len(),
        })
    }

    pub fn poll_relay_messages(
        &mut self,
        peer_id: &str,
    ) -> Result<Vec<RelayMessageEnvelopeModel>, ApiErrorModel> {
        self.prune_expired();

        let mut messages = Vec::new();
        for session in self.relay_sessions.values_mut() {
            if peer_id == session.source_peer_id {
                let drained = drain_messages(&mut messages, &mut session.pending_for_source);
                if drained > 0 {
                    session.source_attached = true;
                    session.expires_at = Instant::now() + self.relay_session_timeout;
                }
            } else if peer_id == session.target_peer_id {
                let drained = drain_messages(&mut messages, &mut session.pending_for_target);
                if drained > 0 {
                    session.target_attached = true;
                    session.expires_at = Instant::now() + self.relay_session_timeout;
                }
            }
        }

        self.relay_sessions
            .retain(|_, session| !relay_session_is_finished(session));

        Ok(messages)
    }

    pub fn prune_expired(&mut self) {
        let now = Instant::now();
        self.peers.retain(|_, peer| peer.expires_at > now);
        self.relay_sessions
            .retain(|_, session| session.expires_at > now);
    }
}

fn drain_messages(
    output: &mut Vec<RelayMessageEnvelopeModel>,
    queue: &mut VecDeque<RelayMessageEnvelopeModel>,
) -> usize {
    let mut drained = 0;
    while let Some(message) = queue.pop_front() {
        output.push(message);
        drained += 1;
    }
    drained
}

fn relay_session_is_finished(session: &RelaySessionModel) -> bool {
    session.source_attached
        && session.target_attached
        && session.pending_for_source.is_empty()
        && session.pending_for_target.is_empty()
}

fn apply_observed_reachability(
    mut reachability: PeerReachabilityModel,
    observed_remote_addr: Option<std::net::SocketAddr>,
) -> PeerReachabilityModel {
    let Some(observed_remote_addr) = observed_remote_addr else {
        return reachability;
    };

    if let Some(observed) = observed_address_candidate(&reachability, observed_remote_addr) {
        if !reachability
            .observed_addresses
            .iter()
            .any(|existing| existing.base_url == observed.base_url)
        {
            reachability.observed_addresses.push(observed);
        }
    }

    reachability
}

fn observed_address_candidate(
    reachability: &PeerReachabilityModel,
    observed_remote_addr: std::net::SocketAddr,
) -> Option<ReachabilityAddressModel> {
    if reachability.mode == DirectReachabilityModeModel::RelayOnly {
        return None;
    }

    let bind_port = reachability
        .bind_address
        .as_ref()
        .and_then(|candidate| candidate.base_url.rsplit(':').next())
        .and_then(|port| port.parse::<u16>().ok())?;
    let observed_ip = observed_remote_addr.ip();

    Some(ReachabilityAddressModel {
        base_url: format!("http://{}:{}", observed_ip, bind_port),
        scope: observed_scope(observed_ip),
        source: ReachabilitySourceModel::HubObserved,
        transport_protocol: TransportProtocolEnum::DirectTcp,
        confidence: ReachabilityConfidenceModel::Low,
        address_hint: observed_ip.to_string(),
    })
}

fn observed_scope(ip: IpAddr) -> ReachabilityScopeModel {
    match ip {
        IpAddr::V4(ip) if ip.is_private() || ip.is_link_local() || ip.is_loopback() => {
            ReachabilityScopeModel::Lan
        }
        IpAddr::V6(ip) if ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback() => {
            ReachabilityScopeModel::Lan
        }
        _ => ReachabilityScopeModel::Public,
    }
}

fn select_direct_candidates(
    source: &PeerPresenceModel,
    target: &PeerPresenceModel,
) -> Option<(Vec<ReachabilityAddressModel>, String)> {
    if source.reachability.mode == DirectReachabilityModeModel::RelayOnly
        || target.reachability.mode == DirectReachabilityModeModel::RelayOnly
    {
        return None;
    }
    if source.reachability.nat_behavior == NatBehaviorModel::Symmetric {
        return None;
    }
    if target.reachability.nat_behavior == NatBehaviorModel::Symmetric {
        return None;
    }

    let public_candidates: Vec<_> = target
        .reachability
        .advertised_addresses
        .iter()
        .filter(|candidate| {
            candidate.transport_protocol == TransportProtocolEnum::DirectTcp
        })
        .filter(|candidate| candidate.scope == ReachabilityScopeModel::Public)
        .cloned()
        .collect();
    if !public_candidates.is_empty() && target.reachability.message_endpoint.is_some() {
        return Some((
            dedupe_candidates(public_candidates),
            "target has externally scoped reachability evidence".to_string(),
        ));
    }

    let source_public_hints = source_public_hints(source);
    let same_network_candidates: Vec<_> = target
        .reachability
        .advertised_addresses
        .iter()
        .filter(|candidate| {
            candidate.transport_protocol == TransportProtocolEnum::DirectTcp
        })
        .filter(|candidate| candidate.scope == ReachabilityScopeModel::Lan)
        .filter(|candidate| source_public_hints.contains(&candidate.address_hint))
        .cloned()
        .collect();
    if !same_network_candidates.is_empty() && target.reachability.message_endpoint.is_some() {
        return Some((
            dedupe_candidates(same_network_candidates),
            "peers appear to share the same local network reachability hint".to_string(),
        ));
    }

    None
}

fn source_public_hints(source: &PeerPresenceModel) -> Vec<String> {
    source
        .reachability
        .advertised_addresses
        .iter()
        .chain(source.reachability.observed_addresses.iter())
        .map(|candidate| candidate.address_hint.clone())
        .collect()
}

fn dedupe_candidates(candidates: Vec<ReachabilityAddressModel>) -> Vec<ReachabilityAddressModel> {
    let mut unique = Vec::new();
    for candidate in candidates {
        if !unique
            .iter()
            .any(|existing: &ReachabilityAddressModel| existing.base_url == candidate.base_url)
        {
            unique.push(candidate);
        }
    }
    unique
}

fn relay_decision_reason(
    source: &PeerPresenceModel,
    target: &PeerPresenceModel,
    prefer_direct: bool,
) -> String {
    if !prefer_direct {
        return "direct delivery was not requested".to_string();
    }
    if source.reachability.mode == DirectReachabilityModeModel::RelayOnly {
        return "source is configured for relay-only delivery".to_string();
    }
    if target.reachability.mode == DirectReachabilityModeModel::RelayOnly {
        return "target is configured for relay-only delivery".to_string();
    }
    if source.reachability.nat_behavior == NatBehaviorModel::Symmetric {
        return "source is behind symmetric NAT; direct hole punching is not credible".to_string();
    }
    if target.reachability.nat_behavior == NatBehaviorModel::Symmetric {
        return "target is behind symmetric NAT; direct hole punching is not credible".to_string();
    }
    if target.reachability.message_endpoint.is_none() {
        return "target has no shared message endpoint".to_string();
    }
    "target has no verified public direct TCP candidate or same-LAN path".to_string()
}

#[cfg(test)]
mod tests {
    use super::ServerStateModel;
    use crate::model::{
        connect_model::{ConnectDecisionModel, ConnectRequestModel},
        message_model::ChatMessageEnvelopeModel,
        peer_model::{
            DirectReachabilityModeModel, NatBehaviorModel, PeerReachabilityModel,
            ReachabilityAddressModel, ReachabilityConfidenceModel, ReachabilityScopeModel,
            ReachabilitySourceModel, RegisterPeerRequestModel, TransportProtocolEnum,
        },
        relay_model::RelaySendRequestModel,
    };
    use std::time::Duration;
    use uuid::Uuid;

    fn register_request_model(peer_id: &str) -> RegisterPeerRequestModel {
        RegisterPeerRequestModel {
            peer_id: peer_id.to_string(),
            device_key: format!("device-key-{peer_id}"),
            display_name: Some(format!("Peer {peer_id}")),
            reachability: PeerReachabilityModel {
                mode: DirectReachabilityModeModel::PublicDirect,
                nat_behavior: NatBehaviorModel::Unknown,
                message_endpoint: Some("/message".to_string()),
                bind_address: Some(candidate(peer_id, ReachabilityScopeModel::Public)),
                advertised_addresses: vec![candidate(peer_id, ReachabilityScopeModel::Public)],
                observed_addresses: Vec::new(),
            },
        }
    }

    fn candidate(peer_id: &str, scope: ReachabilityScopeModel) -> ReachabilityAddressModel {
        ReachabilityAddressModel {
            base_url: format!("http://{peer_id}.example:7443"),
            scope,
            source: ReachabilitySourceModel::Manual,
            transport_protocol: TransportProtocolEnum::DirectTcp,
            confidence: ReachabilityConfidenceModel::High,
            address_hint: format!("{peer_id}.example"),
        }
    }

    #[test]
    fn connect_prefers_direct_when_both_peers_support_it() {
        let mut state = ServerStateModel::new(Duration::from_secs(30), Duration::from_secs(60));
        state.register_peer(register_request_model("alice"), None);
        state.register_peer(register_request_model("bob"), None);

        let decision = state
            .connect(ConnectRequestModel {
                source_peer_id: "alice".to_string(),
                target_peer_id: "bob".to_string(),
                prefer_direct: true,
            })
            .expect("connect should succeed");

        match decision {
            ConnectDecisionModel::Direct { direct } => {
                assert_eq!(direct.peer_id, "bob");
                assert_eq!(direct.message_endpoint.as_deref(), Some("/message"));
                assert!(!direct.reachability_candidates.is_empty());
            }
            ConnectDecisionModel::Relay { .. } => panic!("expected direct connection"),
        }
    }

    #[test]
    fn connect_falls_back_to_relay_when_direct_is_unavailable() {
        let mut state = ServerStateModel::new(Duration::from_secs(30), Duration::from_secs(60));
        let mut alice = register_request_model("alice");
        alice.reachability.mode = DirectReachabilityModeModel::RelayOnly;
        alice.reachability.advertised_addresses.clear();
        state.register_peer(alice, None);
        state.register_peer(register_request_model("bob"), None);

        let decision = state
            .connect(ConnectRequestModel {
                source_peer_id: "alice".to_string(),
                target_peer_id: "bob".to_string(),
                prefer_direct: true,
            })
            .expect("connect should succeed");

        match decision {
            ConnectDecisionModel::Relay { relay } => {
                assert_eq!(relay.source_peer_id, "alice");
                assert_eq!(relay.target_peer_id, "bob");
                assert_eq!(state.relay_sessions.len(), 1);
            }
            ConnectDecisionModel::Direct { .. } => panic!("expected relay connection"),
        }
    }

    #[test]
    fn connect_does_not_treat_hub_observed_tcp_address_as_public_direct() {
        let mut state = ServerStateModel::new(Duration::from_secs(30), Duration::from_secs(60));
        let mut alice = register_request_model("alice");
        alice.reachability.mode = DirectReachabilityModeModel::UnknownExternal;
        alice.reachability.advertised_addresses = vec![candidate("alice", ReachabilityScopeModel::Lan)];
        state.register_peer(alice, Some("203.0.113.10:45678".parse().expect("observed source")));

        let mut bob = register_request_model("bob");
        bob.reachability.mode = DirectReachabilityModeModel::UnknownExternal;
        bob.reachability.advertised_addresses = vec![candidate("bob", ReachabilityScopeModel::Lan)];
        state.register_peer(bob, Some("203.0.113.11:45679".parse().expect("observed source")));

        let decision = state
            .connect(ConnectRequestModel {
                source_peer_id: "alice".to_string(),
                target_peer_id: "bob".to_string(),
                prefer_direct: true,
            })
            .expect("connect should succeed");

        match decision {
            ConnectDecisionModel::Relay { relay } => {
                assert!(relay
                    .decision_reason
                    .contains("no verified public direct TCP candidate"));
            }
            ConnectDecisionModel::Direct { .. } => {
                panic!("hub-observed public TCP address should not enable direct delivery")
            }
        }
    }

    #[test]
    fn expired_peers_are_removed_from_lookup() {
        let mut state = ServerStateModel::new(Duration::from_millis(1), Duration::from_secs(60));
        state.register_peer(register_request_model("alice"), None);
        std::thread::sleep(Duration::from_millis(5));

        assert!(state.lookup_peer("alice").is_none());
    }

    #[test]
    fn heartbeat_extends_peer_presence() {
        let mut state = ServerStateModel::new(Duration::from_millis(20), Duration::from_secs(60));
        state.register_peer(register_request_model("alice"), None);
        std::thread::sleep(Duration::from_millis(10));

        let before = state
            .lookup_peer("alice")
            .expect("peer should still be online");
        state
            .heartbeat("alice")
            .expect("heartbeat should refresh lease");
        std::thread::sleep(Duration::from_millis(15));

        let after = state.lookup_peer("alice");
        assert!(after.is_some());
        assert!(
            after
                .expect("peer should still be online")
                .expires_at_unix_ms
                >= before.expires_at_unix_ms
        );
    }

    #[test]
    fn relay_send_and_poll_deliver_to_the_other_peer() {
        let mut state = ServerStateModel::new(Duration::from_secs(30), Duration::from_secs(60));
        let mut alice = register_request_model("alice");
        alice.reachability.mode = DirectReachabilityModeModel::RelayOnly;
        alice.reachability.advertised_addresses.clear();
        state.register_peer(alice, None);
        state.register_peer(register_request_model("bob"), None);

        let decision = state
            .connect(ConnectRequestModel {
                source_peer_id: "alice".to_string(),
                target_peer_id: "bob".to_string(),
                prefer_direct: false,
            })
            .expect("connect should succeed");

        let relay_session_id = match decision {
            ConnectDecisionModel::Relay { relay } => relay.relay_session_id,
            ConnectDecisionModel::Direct { .. } => panic!("expected relay decision"),
        };

        state
            .attach_relay_peer(relay_session_id, "alice")
            .expect("attach alice");
        state
            .relay_send(RelaySendRequestModel {
                relay_session_id,
                peer_id: "alice".to_string(),
                payload: ChatMessageEnvelopeModel {
                    from_peer_id: "alice".to_string(),
                    to_peer_id: "bob".to_string(),
                    message: "hello bob".to_string(),
                    sent_at_unix_ms: 123,
                },
            })
            .expect("relay send");

        let messages = state.poll_relay_messages("bob").expect("poll bob");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].relay_session_id, relay_session_id);
        assert_eq!(messages[0].payload.message, "hello bob");
        assert!(state.poll_relay_messages("bob").expect("poll again").is_empty());
    }

    #[test]
    fn attach_rejects_unknown_peer() {
        let mut state = ServerStateModel::new(Duration::from_secs(30), Duration::from_secs(60));
        state.register_peer(register_request_model("alice"), None);
        state.register_peer(register_request_model("bob"), None);

        let relay_session_id = match state
            .connect(ConnectRequestModel {
                source_peer_id: "alice".to_string(),
                target_peer_id: "bob".to_string(),
                prefer_direct: false,
            })
            .expect("connect should succeed")
        {
            ConnectDecisionModel::Relay { relay } => relay.relay_session_id,
            ConnectDecisionModel::Direct { .. } => panic!("expected relay decision"),
        };

        let error = state
            .attach_relay_peer(relay_session_id, "mallory")
            .expect_err("unknown peer should fail");
        assert_eq!(error.status, axum::http::StatusCode::BAD_REQUEST);
        assert!(error.message.contains("mallory"));
        assert_ne!(relay_session_id, Uuid::nil());
    }

    #[test]
    fn drained_relay_sessions_are_cleaned_up_after_delivery() {
        let mut state = ServerStateModel::new(Duration::from_secs(30), Duration::from_secs(60));
        let mut alice = register_request_model("alice");
        let mut bob = register_request_model("bob");
        alice.reachability.mode = DirectReachabilityModeModel::RelayOnly;
        bob.reachability.mode = DirectReachabilityModeModel::RelayOnly;
        alice.reachability.advertised_addresses.clear();
        bob.reachability.advertised_addresses.clear();
        state.register_peer(alice, None);
        state.register_peer(bob, None);

        let relay_session_id = match state
            .connect(ConnectRequestModel {
                source_peer_id: "alice".to_string(),
                target_peer_id: "bob".to_string(),
                prefer_direct: false,
            })
            .expect("connect should succeed")
        {
            ConnectDecisionModel::Relay { relay } => relay.relay_session_id,
            ConnectDecisionModel::Direct { .. } => panic!("expected relay decision"),
        };

        state
            .attach_relay_peer(relay_session_id, "alice")
            .expect("attach alice");
        state
            .relay_send(RelaySendRequestModel {
                relay_session_id,
                peer_id: "alice".to_string(),
                payload: ChatMessageEnvelopeModel {
                    from_peer_id: "alice".to_string(),
                    to_peer_id: "bob".to_string(),
                    message: "hello bob".to_string(),
                    sent_at_unix_ms: 123,
                },
            })
            .expect("relay send");

        let messages = state.poll_relay_messages("bob").expect("poll bob");
        assert_eq!(messages.len(), 1);
        assert!(state.relay_sessions.is_empty());
    }
}
