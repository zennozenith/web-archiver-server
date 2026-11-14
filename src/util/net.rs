//! IP/CIDR helpers used by the trusted-proxy logic.

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

use serde::{Deserialize, Deserializer, de};

/// One CIDR block, e.g. `10.0.0.0/8` or `2001:db8::/32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub(crate) fn contains(&self, ip: IpAddr) -> bool {
        // A dual-stack listener reports IPv4 peers as IPv4-mapped IPv6
        // (`::ffff:10.0.0.1`). Canonicalize both sides so an IPv4 CIDR matches.
        match (self.network.to_canonical(), ip.to_canonical()) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => {
                mask_v4(network, self.prefix) == mask_v4(ip, self.prefix)
            }
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                mask_v6(network, self.prefix) == mask_v6(ip, self.prefix)
            }
            _ => false,
        }
    }
}

fn mask_v4(ip: Ipv4Addr, prefix: u8) -> u32 {
    let bits = u32::from(ip);
    if prefix == 0 {
        0
    } else {
        let shift = 32_u32.saturating_sub(u32::from(prefix));
        bits & u32::MAX.checked_shl(shift).unwrap_or(0)
    }
}

fn mask_v6(ip: Ipv6Addr, prefix: u8) -> u128 {
    let bits = u128::from(ip);
    if prefix == 0 {
        0
    } else {
        let shift = 128_u32.saturating_sub(u32::from(prefix));
        bits & u128::MAX.checked_shl(shift).unwrap_or(0)
    }
}

impl FromStr for Cidr {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (address, prefix) = value
            .split_once('/')
            .ok_or_else(|| format!("`{value}` is not address/prefix CIDR notation"))?;
        let network: IpAddr = address
            .trim()
            .parse()
            .map_err(|_| format!("invalid IP address `{address}`"))?;
        let prefix: u8 = prefix
            .trim()
            .parse()
            .map_err(|_| format!("invalid prefix length `{prefix}`"))?;
        let max = if network.is_ipv4() { 32 } else { 128 };
        if prefix > max {
            return Err(format!(
                "prefix length {prefix} is too large for {network} (max {max})"
            ));
        }
        Ok(Self { network, prefix })
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

impl<'de> Deserialize<'de> for Cidr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_containment_respects_prefix() {
        let cidr: Cidr = "10.0.0.0/8".parse().expect("valid cidr");
        assert!(cidr.contains("10.1.2.3".parse().expect("ip")));
        assert!(!cidr.contains("11.1.2.3".parse().expect("ip")));
    }

    #[test]
    fn ipv6_containment_respects_prefix() {
        let cidr: Cidr = "2001:db8::/32".parse().expect("valid cidr");
        assert!(cidr.contains("2001:db8::1".parse().expect("ip")));
        assert!(!cidr.contains("2001:db9::1".parse().expect("ip")));
    }

    #[test]
    fn mismatched_families_never_match() {
        let cidr: Cidr = "127.0.0.0/8".parse().expect("valid cidr");
        assert!(!cidr.contains("::1".parse().expect("ip")));
    }

    #[test]
    fn zero_prefix_matches_everything_of_its_family() {
        let cidr: Cidr = "0.0.0.0/0".parse().expect("valid cidr");
        assert!(cidr.contains("203.0.113.9".parse().expect("ip")));
        assert!(!cidr.contains("::1".parse().expect("ip")));
    }

    #[test]
    fn out_of_range_prefix_is_rejected() {
        assert!("10.0.0.0/33".parse::<Cidr>().is_err());
        assert!("::/129".parse::<Cidr>().is_err());
        assert!("10.0.0.0".parse::<Cidr>().is_err());
    }
}
