//! Tests of the connection against a stand-in gateway on this computer, so
//! the remote path (pairing, device tokens, pinned certificates) is checked
//! without needing a second machine.

use crate::gateway::*;
use crate::identity::{device_auth_payload_v3, DeviceIdentity};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::{accept, Message, WebSocket};

fn target(url: String, device_token: Option<&str>) -> Target {
    Target {
        url,
        secret: Some(Secret::Token("shared-secret".into())),
        device_token: device_token.map(str::to_string),
        route: "test".into(),
        tls_pin: None,
        ssh: None,
        scopes: vec!["operator.read".into(), "operator.write".into()],
    }
}

/// Sends the challenge, reads connect, and checks the device proof.
fn handshake<S: Read + Write>(ws: &mut WebSocket<S>) -> Value {
    let nonce = "nonce-abc";
    let ts = 1_700_000_000_000i64;
    ws.send(Message::text(
        json!({"type":"event","event":"connect.challenge","payload":{"nonce":nonce,"ts":ts}}).to_string(),
    ))
    .unwrap();
    let frame = next_text(ws);
    assert_eq!(frame["method"], "connect");
    let p = &frame["params"];
    assert_eq!(p["minProtocol"], PROTOCOL_VERSION);
    assert_eq!(p["device"]["nonce"], nonce);
    assert_eq!(p["device"]["signedAt"], ts);

    // The signature must match what OpenClaw recomputes on its side.
    let token = p["auth"]["token"].as_str().or(p["auth"]["deviceToken"].as_str()).unwrap_or("");
    let scopes: Vec<String> = p["scopes"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
    let payload = device_auth_payload_v3(
        p["device"]["id"].as_str().unwrap(),
        p["client"]["id"].as_str().unwrap(),
        p["client"]["mode"].as_str().unwrap(),
        p["role"].as_str().unwrap(),
        &scopes,
        ts,
        token,
        nonce,
        p["client"]["platform"].as_str().unwrap(),
        p["client"]["deviceFamily"].as_str().unwrap(),
    );
    let key = URL_SAFE_NO_PAD.decode(p["device"]["publicKey"].as_str().unwrap()).unwrap();
    let key = VerifyingKey::from_bytes(key.as_slice().try_into().unwrap()).unwrap();
    let sig = URL_SAFE_NO_PAD.decode(p["device"]["signature"].as_str().unwrap()).unwrap();
    key.verify(payload.as_bytes(), &Signature::from_slice(&sig).unwrap())
        .expect("device signature");
    frame
}

fn next_text<S: Read + Write>(ws: &mut WebSocket<S>) -> Value {
    loop {
        if let Message::Text(t) = ws.read().unwrap() {
            return serde_json::from_str(t.as_str()).unwrap();
        }
    }
}

fn hello(id: &Value, device_token: Option<&str>) -> String {
    let mut auth = json!({"role":"operator","scopes":["operator.read","operator.write"]});
    if let Some(t) = device_token {
        auth["deviceToken"] = json!(t);
    }
    json!({"type":"res","id":id,"ok":true,"payload":{
        "type":"hello-ok","protocol":4,
        "server":{"version":"test","connId":"c1"},
        "features":{"methods":["chat.send"],"events":["chat"]},
        "snapshot":{},"auth":auth,
        "policy":{"maxPayload":1000000,"maxBufferedBytes":1000000,"tickIntervalMs":15000}
    }})
    .to_string()
}

fn refusal(id: &Value, gw_code: &str, detail_code: &str, request: &str) -> String {
    json!({"type":"res","id":id,"ok":false,"error":{
        "code":gw_code,"message":"nope","details":{"code":detail_code,"requestId":request}
    }})
    .to_string()
}

fn collect_until(rx: &Receiver<NetEvent>, what: &str, seconds: u64, done: impl Fn(&NetEvent) -> bool) -> Vec<NetEvent> {
    let until = Instant::now() + Duration::from_secs(seconds);
    let mut seen = vec![];
    while Instant::now() < until {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
            let stop = done(&ev);
            seen.push(ev);
            if stop {
                return seen;
            }
        }
    }
    panic!("timed out waiting for {what}");
}

fn identity() -> DeviceIdentity {
    DeviceIdentity::from_seed(&[9u8; 32])
}

#[test]
fn pairing_then_connects_saves_token_and_streams() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        // First attempt: this device isn't approved yet.
        let (s, _) = listener.accept().unwrap();
        let mut ws = accept(s).unwrap();
        let f = handshake(&mut ws);
        assert_eq!(f["params"]["auth"]["token"], "shared-secret");
        ws.send(Message::text(refusal(&f["id"], "NOT_PAIRED", "PAIRING_REQUIRED", "req-1"))).unwrap();
        let _ = ws.close(None);
        let _ = ws.flush();
        drop(ws);

        // Second attempt: approved.
        let (s, _) = listener.accept().unwrap();
        let mut ws = accept(s).unwrap();
        let f = handshake(&mut ws);
        ws.send(Message::text(hello(&f["id"], Some("device-token-1")))).unwrap();
        let req = next_text(&mut ws);
        assert_eq!(req["method"], "chat.send");
        assert_eq!(req["params"]["message"], "hola");
        ws.send(Message::text(json!({"type":"res","id":req["id"],"ok":true,"payload":{"runId":"run-1"}}).to_string())).unwrap();
        ws.send(Message::text(json!({"type":"event","event":"chat","seq":1,"payload":{
            "runId":"run-1","sessionKey":"s","state":"delta","deltaText":"ho"}}).to_string())).unwrap();
        // Hold the connection open until the client closes.
        loop {
            match ws.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    let (mut client, rx) = Client::start(target(url, None), identity(), || {});
    let seen = collect_until(&rx, "the pairing request", 10, |e| matches!(e, NetEvent::State(ConnState::Pairing { .. })));
    assert!(seen.iter().any(|e| matches!(e, NetEvent::State(ConnState::Pairing { request_id }) if request_id == "req-1")));

    let seen = collect_until(&rx, "the connection", 15, |e| matches!(e, NetEvent::Hello(_)));
    assert!(seen.iter().any(|e| matches!(e, NetEvent::State(ConnState::Connected))));
    assert!(seen.iter().any(|e| matches!(e, NetEvent::DeviceToken { token, .. } if token == "device-token-1")));

    let id = client.request("chat.send", json!({"sessionKey":"s","message":"hola"}));
    let seen = collect_until(&rx, "the send answer", 5, |e| matches!(e, NetEvent::Response { .. }));
    assert!(seen.iter().any(|e| matches!(e, NetEvent::Response { id: i, result: Ok(_) } if *i == id)));
    let seen = collect_until(&rx, "the live text", 5, |e| matches!(e, NetEvent::Event { .. }));
    assert!(seen.iter().any(|e| matches!(e, NetEvent::Event { event, payload } if event == "chat" && payload["deltaText"] == "ho")));

    client.stop();
    server.join().unwrap();
}

#[test]
fn refused_device_token_falls_back_to_the_shared_secret() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (s, _) = listener.accept().unwrap();
        let mut ws = accept(s).unwrap();
        let f = handshake(&mut ws);
        assert_eq!(f["params"]["auth"]["deviceToken"], "old-device-token");
        assert!(f["params"]["auth"].get("token").is_none());
        ws.send(Message::text(refusal(&f["id"], "UNAUTHORIZED", "AUTH_DEVICE_TOKEN_MISMATCH", ""))).unwrap();
        let _ = ws.close(None);
        let _ = ws.flush();
        drop(ws);

        let (s, _) = listener.accept().unwrap();
        let mut ws = accept(s).unwrap();
        let f = handshake(&mut ws);
        assert_eq!(f["params"]["auth"]["token"], "shared-secret");
        assert!(f["params"]["auth"].get("deviceToken").is_none());
        ws.send(Message::text(hello(&f["id"], None))).unwrap();
        loop {
            match ws.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    let (client, rx) = Client::start(target(url, Some("old-device-token")), identity(), || {});
    let seen = collect_until(&rx, "the hello", 15, |e| matches!(e, NetEvent::Hello(_)));
    assert!(seen.iter().any(|e| matches!(e, NetEvent::ForgetDeviceToken { .. })));
    client.stop();
    server.join().unwrap();
}

#[test]
fn wrong_secret_stops_and_says_why() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (s, _) = listener.accept().unwrap();
        let mut ws = accept(s).unwrap();
        let f = handshake(&mut ws);
        ws.send(Message::text(refusal(&f["id"], "UNAUTHORIZED", "AUTH_TOKEN_MISMATCH", ""))).unwrap();
        let _ = ws.close(None);
        let _ = ws.flush();
    });
    let (client, rx) = Client::start(target(url, None), identity(), || {});
    let seen = collect_until(&rx, "the refusal", 10, |e| matches!(e, NetEvent::State(ConnState::AuthFailed(_))));
    assert!(seen.iter().any(|e| matches!(e, NetEvent::State(ConnState::AuthFailed(m)) if m.contains("clave"))));
    client.stop();
    server.join().unwrap();
}

#[test]
fn pinned_certificate_connects_and_a_wrong_pin_is_refused() {
    use sha2::{Digest, Sha256};
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let der = cert.cert.der().to_vec();
    let key = cert.key_pair.serialize_der();
    let pin = crate::identity::hex(&Sha256::digest(&der));

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(der)],
            rustls::pki_types::PrivateKeyDer::try_from(key).unwrap(),
        )
        .unwrap();
    let config = Arc::new(config);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            let conn = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, stream);
            // A refused client aborts the handshake; both outcomes are fine here.
            if tls.conn.complete_io(&mut tls.sock).is_ok() {
                if let Ok(mut ws) = accept(tls) {
                    let _ = ws.read();
                }
            }
        }
    });

    let url = format!("wss://localhost:{port}");
    let wrong = "ab".repeat(32);
    assert!(open_socket(&url, Some(&wrong)).is_err(), "a wrong pin must be refused");
    let mut good = open_socket(&url, Some(&pin)).expect("the right pin connects");
    let _ = good.close(None);
    let _ = good.flush();
    drop(good);
    server.join().unwrap();
}
