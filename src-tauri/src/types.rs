//! What kind of thing a directive holds.
//!
//! This is the hand-written layer — the one part not derived from UnrealIRCd's
//! source or wiki, because the documentation implies these types without ever
//! writing them down. Keep it small and honest: when we aren't sure, say Text
//! and let the value through untouched. A control that refuses valid config is
//! worse than a plain box.

use serde::{Deserialize, Serialize};

// Deserialize too: the kind is cached in the docs pack on disk, so it has to
// survive a round trip through JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Text,
    Number,
    /// `200k`, `20M`
    Size,
    /// `5m`, `90`, `2h`
    Duration,
    /// `3:60` — three per sixty seconds
    Rate,
    /// `6697`, or a range `6667-6669`
    Port,
    YesNo,
    /// A hostmask, `*@*.example.net`
    Mask,
    /// An address or CIDR range
    Ip,
    /// A block holding bare items rather than name/value pairs
    List,
    /// A password, with an auth-type
    Password,
    /// `+cFfkQGqso`
    Modes,
}

/// One criterion inside a `mask { }` or a `security-group { }`.
#[derive(Debug, Clone, Serialize)]
pub struct Criterion {
    pub name: &'static str,
    /// What it means, in words
    pub about: &'static str,
    pub kind: Kind,
    /// True for the `exclude-` half of the pair
    pub excludes: bool,
}

const fn c(name: &'static str, about: &'static str, kind: Kind) -> Criterion {
    Criterion { name, about, kind, excludes: false }
}

/// The matcher vocabulary. `mask { }` and `security-group { }` share it, which
/// the wiki says outright: "same fields as in a security-group block".
pub const CRITERIA: &[Criterion] = &[
    c("mask", "Their hostmask, like *.example.net", Kind::Mask),
    c("include-mask", "Their hostmask, counted as a match on its own", Kind::Mask),
    c("ip", "Their address, or a CIDR range like 127.0.0.0/8", Kind::Ip),
    c("identified", "They are logged in to services", Kind::YesNo),
    c("webirc", "They came in through a web gateway", Kind::YesNo),
    c("websocket", "They are using WebSockets", Kind::YesNo),
    c("tls", "They are on an encrypted connection", Kind::YesNo),
    c("reputation-score", "Their reputation is at least this", Kind::Number),
    c("connect-time", "They have been connected at least this long", Kind::Duration),
    c("security-group", "They are in one of these groups", Kind::List),
    c("account", "Their services account is one of these", Kind::List),
    c("country", "Their country, as GeoIP sees it", Kind::List),
    c("asn", "Their network's ASN, as GeoIP sees it", Kind::List),
    c("realname", "Their real name matches", Kind::List),
    c("certfp", "Their certificate fingerprint is one of these", Kind::List),
    c("channel", "They are in one of these channels", Kind::List),
    c("destination", "Where they were being sent", Kind::List),
    c("server-port", "The port they connected to", Kind::List),
    c("rule", "A crule expression", Kind::Text),
];

pub fn criterion(name: &str) -> Option<&'static Criterion> {
    let bare = name.strip_prefix("exclude-").unwrap_or(name);
    CRITERIA.iter().find(|x| x.name == bare)
}

/// Is this directive a matcher — a `mask { }` or an `except { }` that takes
/// the criteria above?
///
/// Deliberately conservative. `mask` always is. `except` only in the places the
/// wiki lists, because `except` is also a top-level block that merely
/// *contains* a mask, and `set::spamfilter::except` is a plain list of targets.
pub fn is_matcher(path: &str) -> bool {
    let last = path.rsplit("::").next().unwrap_or(path);
    // conf.c spells this two ways and means the same thing by both: every site
    // reads `!strcmp(name, "mask") || !strcmp(name, "match")` and hands off to
    // the same conf_match_block(). Knowing only one of them leaves `match { }`
    // looking like a plain value with no criteria behind it.
    if (last == "mask" || last == "match") && path.contains("::") {
        return true;
    }
    // `security-group { ip 1.2.3.4; }` puts criteria straight in the block, so
    // the block itself is a matcher, not only its children.
    if path == "security-group" || path.starts_with("security-group::") {
        return true;
    }
    const EXCEPT_MATCHERS: &[&str] = &[
        "connthrottle::except",
        "blacklist::except",
        "set::antimixedutf8::except",
        "set::antirandom::except",
        "set::connthrottle::except",
    ];
    if EXCEPT_MATCHERS.contains(&path) {
        return true;
    }
    // set::restrict-commands::<command>::except
    path.starts_with("set::restrict-commands::") && path.ends_with("::except")
}

/// The value type for a directive, by path. Falls back to matching on the last
/// segment, since names like `sendq` and `ban-time` mean the same thing
/// wherever they turn up.
pub fn kind_of(path: &str) -> Kind {
    let last = path.rsplit("::").next().unwrap_or(path);

    if let Some(cr) = criterion(last) {
        // Only inside a matcher; `account` elsewhere is something else.
        if path.contains("::") && is_matcher(path.rsplit_once("::").map(|x| x.0).unwrap_or("")) {
            return cr.kind;
        }
    }

    match last {
        "sendq" | "recvq" | "maxbanlength" | "amount" => Kind::Size,
        "pingfreq" | "connfreq" | "ban-time" | "connect-time" | "handshake-timeout"
        | "sasl-timeout" | "default-bantime" | "recheck-time" | "recheck-time-first"
        | "anti-spam-quit-message-time" | "new-user-every" | "handshake-delay" => Kind::Duration,
        "connect-flood" | "nick-flood" | "join-flood" | "away-flood" | "invite-flood"
        | "knock-flood" | "channel-privmsg" | "channel-notice" | "channel-tagmsg"
        | "private-privmsg" | "private-notice" | "private-tagmsg" => Kind::Rate,
        "port" | "server-port" => Kind::Port,
        "password" => Kind::Password,
        "modes" | "modes-on-connect" | "modes-on-oper" | "modes-on-join" | "snomask"
        | "snomask-on-oper" | "restrict-usermodes" | "restrict-channelmodes" => Kind::Modes,
        "ip" | "bind-ip" | "spoof-ip" => Kind::Ip,
        "mask" => Kind::Mask,
        "maxclients" | "maxperip" | "global-maxperip" | "maxlogins" | "maxchannelsperuser"
        | "maxbans" | "priority" | "reputation-score" | "nick-length" | "topic-length"
        | "away-length" | "quit-length" | "kick-length" | "min-nick-length"
        | "maxdccallow" | "who-limit" | "silence-limit" | "leaf-depth" | "users" => Kind::Number,
        "identified" | "webirc" | "websocket" | "tls" | "hub" | "leaf" | "auto-login"
        | "public" | "verify-certificate" | "server-notice-colors"
        | "server-notice-show-event" => Kind::YesNo,
        _ => Kind::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_are_matchers_wherever_they_appear() {
        assert!(is_matcher("oper::mask"));
        // `match` is the same thing spelled differently — src/conf.c:4464 and
        // everywhere else accepts either.
        assert!(is_matcher("oper::match"));
        assert!(is_matcher("allow::match"));
        assert!(is_matcher("tld::match"));
        assert!(is_matcher("allow::mask"));
        assert!(is_matcher("link::incoming::mask"));
        assert!(is_matcher("security-group"));
        assert!(is_matcher("security-group::account"));
        assert!(is_matcher("set::restrict-commands::list::except"));
        assert!(is_matcher("blacklist::except"));
    }

    #[test]
    fn except_is_not_a_matcher_everywhere() {
        // A top-level `except ban { }` contains a mask; it isn't one itself.
        assert!(!is_matcher("except"));
        // And this one is a list of targets, not people.
        assert!(!is_matcher("set::spamfilter::except"));
    }

    #[test]
    fn common_value_types() {
        assert_eq!(kind_of("class::sendq"), Kind::Size);
        assert_eq!(kind_of("class::pingfreq"), Kind::Duration);
        assert_eq!(kind_of("set::anti-flood::everyone::connect-flood"), Kind::Rate);
        assert_eq!(kind_of("listen::port"), Kind::Port);
        assert_eq!(kind_of("oper::password"), Kind::Password);
        assert_eq!(kind_of("me::name"), Kind::Text);
    }

    #[test]
    fn criteria_inside_a_matcher_get_their_own_type() {
        assert_eq!(kind_of("oper::mask::identified"), Kind::YesNo);
        assert_eq!(kind_of("oper::mask::certfp"), Kind::List);
        assert_eq!(kind_of("oper::mask::connect-time"), Kind::Duration);
        assert_eq!(kind_of("security-group::tls"), Kind::YesNo);
    }

    #[test]
    fn exclude_variants_resolve_to_their_pair() {
        assert_eq!(criterion("exclude-certfp").map(|c| c.name), Some("certfp"));
        assert_eq!(criterion("exclude-tls").map(|c| c.kind), Some(Kind::YesNo));
    }
}
