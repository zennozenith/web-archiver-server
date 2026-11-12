use std::net::IpAddr;

/// Who is calling, as far as the server can tell. Stored in the login audit
/// trail and on sessions.
#[derive(Debug, Clone)]
pub(crate) struct ClientInfo {
    pub ip: IpAddr,
    pub user_agent: String,
}

/// UAs are attacker-controlled and get stored, so cap the length.
const MAX_UA_LEN: usize = 256;

impl ClientInfo {
    pub(crate) fn new(ip: IpAddr, user_agent: Option<&str>) -> Self {
        let ua = user_agent.unwrap_or("unknown");
        let user_agent = ua.chars().take(MAX_UA_LEN).collect();
        Self { ip, user_agent }
    }
}
