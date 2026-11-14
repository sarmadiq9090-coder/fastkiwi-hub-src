//! fastkiwi-module@1 validation, both directions.

use kiwi_hub::protocol::{
    self, Digits, HubMsg, Lang, MAX_MESSAGE_BYTES, ModuleMsg, ProtoError, Theme, UpdateState,
};

const P: &str = "\"proto\":\"fastkiwi-module@1\"";

fn m(body: &str) -> Result<ModuleMsg, ProtoError> {
    protocol::parse_module_msg(&format!("{{{P},{body}}}"), "instawords")
}
fn h(body: &str) -> Result<HubMsg, ProtoError> {
    protocol::parse_hub_msg(&format!("{{{P},{body}}}"))
}

#[test]
fn module_messages_accepted() {
    assert_eq!(
        m("\"type\":\"ready\",\"module\":\"instawords\",\"version\":\"3.0.0-beta.1+web\"").unwrap(),
        ModuleMsg::Ready {
            module: "instawords".into(),
            version: Some("3.0.0-beta.1+web".into())
        }
    );
    assert_eq!(
        m("\"type\":\"ready\",\"module\":\"instawords\"").unwrap(),
        ModuleMsg::Ready {
            module: "instawords".into(),
            version: None
        }
    );
    assert_eq!(
        m("\"type\":\"open-own-window\"").unwrap(),
        ModuleMsg::OpenOwnWindow {}
    );
}

#[test]
fn module_messages_refused() {
    assert!(matches!(
        m("\"type\":\"ready\",\"module\":\"lemon\""),
        Err(ProtoError::WrongModule { .. })
    ));
    assert!(matches!(
        m("\"type\":\"ready\",\"module\":\"instawords\",\"version\":\"1.0\""),
        Err(ProtoError::BadValue(_))
    ));
    assert!(matches!(
        m("\"type\":\"ready\",\"module\":\"instawords\",\"version\":\"01.0.0\""),
        Err(ProtoError::BadValue(_))
    ));
    assert!(matches!(
        m("\"type\":\"theme\",\"theme\":\"dark\",\"tone\":\"dark\""),
        Err(ProtoError::WrongDirection(_))
    ));
    assert!(matches!(
        m("\"type\":\"update-progress\",\"state\":\"done\""),
        Err(ProtoError::WrongDirection(_))
    ));
    assert!(matches!(
        m("\"type\":\"eval\",\"code\":\"x\""),
        Err(ProtoError::UnknownType(_))
    ));
    assert!(matches!(
        m("\"type\":\"open-own-window\",\"path\":\"C:\\\\x.exe\""),
        Err(ProtoError::BadFields(_))
    ));
    assert!(matches!(
        m("\"type\":\"ready\",\"module\":\"instawords\",\"x\":1"),
        Err(ProtoError::BadFields(_))
    ));
    assert!(matches!(
        m("\"type\":\"ready\""),
        Err(ProtoError::BadFields(_))
    ));
    assert!(matches!(m("\"type\":5"), Err(ProtoError::MissingType)));
    // envelope
    let pm = |t: &str| protocol::parse_module_msg(t, "instawords");
    assert!(matches!(
        pm("{\"type\":\"open-own-window\"}"),
        Err(ProtoError::BadProto)
    ));
    assert!(matches!(
        pm("{\"proto\":\"fastkiwi-module@2\",\"type\":\"open-own-window\"}"),
        Err(ProtoError::BadProto)
    ));
    assert!(matches!(pm("[1,2]"), Err(ProtoError::NotObject)));
    assert!(matches!(
        pm("\"open-own-window\""),
        Err(ProtoError::NotObject)
    ));
    assert!(matches!(pm("{oops"), Err(ProtoError::NotJson)));
    let big = format!(
        "{{{P},\"type\":\"ready\",\"module\":\"instawords\",\"pad\":\"{}\"}}",
        "x".repeat(MAX_MESSAGE_BYTES)
    );
    assert!(matches!(pm(&big), Err(ProtoError::TooLarge(_))));
}

#[test]
fn hub_messages_round_trip() {
    for (t, l, d) in [
        (Theme::Light, Lang::Ar, Digits::Arab),
        (Theme::Midnight, Lang::En, Digits::Latn),
        (Theme::Kiwi, Lang::Ar, Digits::Latn),
    ] {
        for msg in protocol::settings_msgs(t, l, d) {
            let text = protocol::encode_hub_msg(&msg);
            assert!(text.contains("\"proto\":\"fastkiwi-module@1\""), "{text}");
            assert_eq!(protocol::parse_hub_msg(&text).unwrap(), msg);
        }
    }
    let up = HubMsg::UpdateProgress {
        state: UpdateState::Downloading,
        progress: Some(0.45),
    };
    assert_eq!(
        protocol::parse_hub_msg(&protocol::encode_hub_msg(&up)).unwrap(),
        up
    );
    let done = HubMsg::UpdateProgress {
        state: UpdateState::Done,
        progress: None,
    };
    let text = protocol::encode_hub_msg(&done);
    assert!(!text.contains("progress\":"), "{text}");
    assert_eq!(protocol::parse_hub_msg(&text).unwrap(), done);
    let ready = ModuleMsg::Ready {
        module: "instawords".into(),
        version: None,
    };
    assert_eq!(
        protocol::parse_module_msg(&protocol::encode_module_msg(&ready), "instawords").unwrap(),
        ready
    );
}

#[test]
fn hub_messages_refused() {
    assert!(matches!(
        h("\"type\":\"theme\",\"theme\":\"dark\",\"tone\":\"light\""),
        Err(ProtoError::BadValue(_))
    ));
    assert!(matches!(
        h("\"type\":\"theme\",\"theme\":\"neon\",\"tone\":\"dark\""),
        Err(ProtoError::BadFields(_))
    ));
    assert!(matches!(
        h("\"type\":\"lang\",\"lang\":\"ar\",\"dir\":\"ltr\""),
        Err(ProtoError::BadValue(_))
    ));
    assert!(matches!(
        h("\"type\":\"lang\",\"lang\":\"fr\",\"dir\":\"ltr\""),
        Err(ProtoError::BadFields(_))
    ));
    assert!(matches!(
        h("\"type\":\"digits\",\"digits\":\"hanidec\""),
        Err(ProtoError::BadFields(_))
    ));
    assert!(matches!(
        h("\"type\":\"update-progress\",\"state\":\"downloading\""),
        Err(ProtoError::BadValue(_))
    ));
    assert!(matches!(
        h("\"type\":\"update-progress\",\"state\":\"installing\",\"progress\":1.5"),
        Err(ProtoError::BadValue(_))
    ));
    assert!(matches!(
        h("\"type\":\"update-progress\",\"state\":\"installing\",\"progress\":-0.1"),
        Err(ProtoError::BadValue(_))
    ));
    assert!(matches!(
        h("\"type\":\"digits\",\"digits\":\"arab\",\"extra\":true"),
        Err(ProtoError::BadFields(_))
    ));
    assert!(matches!(
        h("\"type\":\"ready\",\"module\":\"instawords\""),
        Err(ProtoError::WrongDirection(_))
    ));
}

#[test]
fn semver() {
    for ok in [
        "0.1.0",
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-0.3.7",
        "1.0.0+20130313144700",
        "1.0.0-beta+exp.sha.5114f85",
    ] {
        assert!(protocol::is_semver(ok), "{ok}");
    }
    for bad in [
        "1",
        "1.0",
        "1.0.0.0",
        "01.0.0",
        "1.0.0-",
        "1.0.0-01",
        "1.0.0+",
        "a.b.c",
        "1.0.0-al pha",
    ] {
        assert!(!protocol::is_semver(bad), "{bad}");
    }
}
