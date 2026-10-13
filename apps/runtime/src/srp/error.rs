use std::fmt;

/// Why a trunk or one of its requests failed.
///
/// [`SrpError::code`] is stable: it surfaces to users as
/// `error.data.storm_error`, so a code is renamed only as a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SrpError {
    /// The relay URL or `server_id` cannot be dialled as given — including a
    /// plaintext `ws://` relay that is not on this machine.
    InvalidTarget(String),
    /// The relay answered and the server is not registered with it
    /// (`server_unreachable` after the relay's 5 s `HELLO` hold).
    ServerOffline,
    /// The relay could not be reached, or dropped the socket.
    RelayDown(String),
    /// Nothing heard from the relay for the silence limit (three heartbeats).
    RelaySilent,
    /// No `READY` within the handshake timeout.
    HandshakeTimeout,
    /// The trunk died while this request was in flight, or the relay said the
    /// server's trunk did.
    TrunkLost,
    /// The server re-registered and its old trunk's drain window ran out.
    TrunkSuperseded,
    /// The origin abandoned the response after its head: the body is
    /// incomplete.
    StreamAborted,
    /// The relay dropped this stream because this client fell behind reading
    /// it (decision 106). Like an abort: the body is incomplete.
    ClientBacklogged,
    /// The server did not acknowledge the stream within the relay's 5 s.
    ServerTimeout,
    /// The relay refused an open or a `HELLO` under its abuse limits.
    RateLimited,
    /// The stream was closed before it produced a response.
    StreamClosed,
    /// A malformed frame, on either side.
    ProtocolError,
    /// The trunk was closed by this side.
    Closed,
    /// A relay code this build does not know.
    Relay(String),
}

impl SrpError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidTarget(_) => "invalid_relay_target",
            Self::ServerOffline => "server_offline",
            Self::RelayDown(_) => "relay_down",
            Self::RelaySilent => "relay_silent",
            Self::HandshakeTimeout => "handshake_timeout",
            Self::TrunkLost => "trunk_lost",
            Self::TrunkSuperseded => "trunk_superseded",
            Self::StreamAborted => "stream_aborted",
            Self::ClientBacklogged => "client_backlogged",
            Self::ServerTimeout => "server_timeout",
            Self::RateLimited => "rate_limited",
            Self::StreamClosed => "stream_closed",
            Self::ProtocolError => "protocol_error",
            Self::Closed => "trunk_closed",
            Self::Relay(_) => "relay_error",
        }
    }

    /// Maps a §6 code the relay sent.
    pub(crate) fn from_relay_code(code: &str) -> Self {
        match code {
            "server_unreachable" => Self::ServerOffline,
            "server_timeout" => Self::ServerTimeout,
            "trunk_lost" => Self::TrunkLost,
            "trunk_superseded" => Self::TrunkSuperseded,
            "stream_aborted" => Self::StreamAborted,
            "client_backlogged" => Self::ClientBacklogged,
            "rate_limited" => Self::RateLimited,
            "stream_closed" => Self::StreamClosed,
            "protocol_error" => Self::ProtocolError,
            other => Self::Relay(other.to_string()),
        }
    }
}

impl fmt::Display for SrpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTarget(why) => write!(f, "cannot dial this relay: {why}"),
            Self::ServerOffline => {
                f.write_str("the relay is up but the server is not connected to it")
            }
            Self::RelayDown(why) => write!(f, "the relay is unavailable: {why}"),
            Self::RelaySilent => f.write_str("the relay stopped answering"),
            Self::HandshakeTimeout => f.write_str("the relay did not answer HELLO in time"),
            Self::TrunkLost => f.write_str("the relay connection was lost mid-request"),
            Self::TrunkSuperseded => f.write_str("the server reconnected to the relay; retry"),
            Self::StreamAborted => f.write_str("the server abandoned the response partway"),
            Self::ClientBacklogged => {
                f.write_str("the relay dropped the response: this client fell behind reading it")
            }
            Self::ServerTimeout => f.write_str("the server did not accept the request in time"),
            Self::RateLimited => f.write_str("the relay is rate-limiting this connection"),
            Self::StreamClosed => f.write_str("the request was closed before a response"),
            Self::ProtocolError => f.write_str("relay protocol error"),
            Self::Closed => f.write_str("the relay connection was closed"),
            Self::Relay(code) => write!(f, "the relay refused: {code}"),
        }
    }
}

impl std::error::Error for SrpError {}
