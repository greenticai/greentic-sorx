use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagerChannel {
    WebChat,
    Teams,
    Slack,
    Webex,
    Web,
    #[default]
    Api,
    Unknown(String),
}

impl ManagerChannel {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "webchat" | "web_chat" => Self::WebChat,
            "teams" | "msteams" | "microsoft_teams" => Self::Teams,
            "slack" => Self::Slack,
            "webex" => Self::Webex,
            "web" => Self::Web,
            "api" => Self::Api,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn capabilities(&self) -> ChannelCapabilities {
        match self {
            Self::WebChat | Self::Web | Self::Api => ChannelCapabilities {
                canonical_adaptive_cards: true,
                supports_submit: true,
                supports_refresh: true,
                supports_dynamic_choices: true,
                supports_svg_image: true,
                supports_rtl_hint: true,
                max_card_size_bytes: None,
                max_actions: None,
            },
            Self::Teams => ChannelCapabilities {
                canonical_adaptive_cards: true,
                supports_submit: true,
                supports_refresh: true,
                supports_dynamic_choices: false,
                supports_svg_image: true,
                supports_rtl_hint: true,
                max_card_size_bytes: Some(28 * 1024),
                max_actions: Some(6),
            },
            Self::Slack | Self::Webex => ChannelCapabilities {
                canonical_adaptive_cards: true,
                supports_submit: true,
                supports_refresh: false,
                supports_dynamic_choices: false,
                supports_svg_image: false,
                supports_rtl_hint: false,
                max_card_size_bytes: Some(20 * 1024),
                max_actions: Some(5),
            },
            Self::Unknown(_) => ChannelCapabilities {
                canonical_adaptive_cards: true,
                supports_submit: false,
                supports_refresh: false,
                supports_dynamic_choices: false,
                supports_svg_image: false,
                supports_rtl_hint: false,
                max_card_size_bytes: Some(16 * 1024),
                max_actions: Some(3),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelCapabilities {
    pub canonical_adaptive_cards: bool,
    pub supports_submit: bool,
    pub supports_refresh: bool,
    pub supports_dynamic_choices: bool,
    pub supports_svg_image: bool,
    pub supports_rtl_hint: bool,
    pub max_card_size_bytes: Option<usize>,
    pub max_actions: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_channel_aliases() {
        let cases = [
            ("webchat", ManagerChannel::WebChat),
            (" web_chat ", ManagerChannel::WebChat),
            ("teams", ManagerChannel::Teams),
            ("msteams", ManagerChannel::Teams),
            ("microsoft_teams", ManagerChannel::Teams),
            ("slack", ManagerChannel::Slack),
            ("webex", ManagerChannel::Webex),
            ("web", ManagerChannel::Web),
            ("api", ManagerChannel::Api),
        ];

        for (raw, expected) in cases {
            assert_eq!(ManagerChannel::parse(raw), expected, "{raw}");
        }
    }

    #[test]
    fn web_like_channels_support_full_card_features() {
        for channel in [
            ManagerChannel::WebChat,
            ManagerChannel::Web,
            ManagerChannel::Api,
        ] {
            let capabilities = channel.capabilities();
            assert!(capabilities.canonical_adaptive_cards);
            assert!(capabilities.supports_submit);
            assert!(capabilities.supports_refresh);
            assert!(capabilities.supports_dynamic_choices);
            assert!(capabilities.supports_svg_image);
            assert!(capabilities.supports_rtl_hint);
            assert_eq!(capabilities.max_card_size_bytes, None);
            assert_eq!(capabilities.max_actions, None);
        }
    }

    #[test]
    fn teams_caps_actions_and_card_size() {
        let capabilities = ManagerChannel::Teams.capabilities();
        assert!(capabilities.canonical_adaptive_cards);
        assert!(capabilities.supports_submit);
        assert!(capabilities.supports_refresh);
        assert!(!capabilities.supports_dynamic_choices);
        assert!(capabilities.supports_svg_image);
        assert!(capabilities.supports_rtl_hint);
        assert_eq!(capabilities.max_card_size_bytes, Some(28 * 1024));
        assert_eq!(capabilities.max_actions, Some(6));
    }

    #[test]
    fn slack_and_webex_use_reduced_card_features() {
        for channel in [ManagerChannel::Slack, ManagerChannel::Webex] {
            let capabilities = channel.capabilities();
            assert!(capabilities.canonical_adaptive_cards);
            assert!(capabilities.supports_submit);
            assert!(!capabilities.supports_refresh);
            assert!(!capabilities.supports_dynamic_choices);
            assert!(!capabilities.supports_svg_image);
            assert!(!capabilities.supports_rtl_hint);
            assert_eq!(capabilities.max_card_size_bytes, Some(20 * 1024));
            assert_eq!(capabilities.max_actions, Some(5));
        }
    }

    #[test]
    fn unknown_channel_degrades_safely() {
        let channel = ManagerChannel::parse("future-channel");
        assert_eq!(
            channel,
            ManagerChannel::Unknown("future-channel".to_string())
        );
        let capabilities = channel.capabilities();
        assert!(capabilities.canonical_adaptive_cards);
        assert!(!capabilities.supports_submit);
        assert!(!capabilities.supports_refresh);
        assert!(!capabilities.supports_dynamic_choices);
        assert!(!capabilities.supports_svg_image);
        assert!(!capabilities.supports_rtl_hint);
        assert_eq!(capabilities.max_card_size_bytes, Some(16 * 1024));
        assert_eq!(capabilities.max_actions, Some(3));
    }
}
