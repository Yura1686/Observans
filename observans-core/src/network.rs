use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListenerKind {
    Loopback,
    Tailscale,
    Lan,
}

impl ListenerKind {
    pub fn label(self) -> &'static str {
        match self {
            ListenerKind::Loopback => "loopback",
            ListenerKind::Tailscale => "tailscale",
            ListenerKind::Lan => "lan",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListenerBinding {
    pub kind: ListenerKind,
    pub addr: SocketAddr,
}

impl ListenerBinding {
    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }
}

#[derive(Clone, Debug)]
pub struct NetworkSnapshot {
    pub lan_enabled: bool,
    pub bindings: Vec<ListenerBinding>,
}

#[derive(Clone, Debug)]
pub struct SharedNetworkPolicy {
    inner: Arc<NetworkPolicyInner>,
}

#[derive(Debug)]
struct NetworkPolicyInner {
    port: u16,
    lan_enabled: AtomicBool,
    lan_tx: watch::Sender<bool>,
    active_bindings: Mutex<Vec<ListenerBinding>>,
}

impl SharedNetworkPolicy {
    pub fn new(port: u16, lan_enabled: bool) -> Self {
        let (lan_tx, _lan_rx) = watch::channel(lan_enabled);
        Self {
            inner: Arc::new(NetworkPolicyInner {
                port,
                lan_enabled: AtomicBool::new(lan_enabled),
                lan_tx,
                active_bindings: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    pub fn lan_enabled(&self) -> bool {
        self.inner.lan_enabled.load(Ordering::SeqCst)
    }

    pub fn subscribe_lan(&self) -> watch::Receiver<bool> {
        self.inner.lan_tx.subscribe()
    }

    pub fn set_lan_enabled(&self, enabled: bool) -> bool {
        let changed = self.inner.lan_enabled.swap(enabled, Ordering::SeqCst) != enabled;
        if changed {
            let _ = self.inner.lan_tx.send(enabled);
        }
        changed
    }

    pub fn toggle_lan(&self) -> bool {
        let next = !self.lan_enabled();
        self.set_lan_enabled(next);
        next
    }

    pub fn snapshot(&self) -> NetworkSnapshot {
        let bindings = self
            .inner
            .active_bindings
            .lock()
            .expect("network policy lock poisoned")
            .clone();
        NetworkSnapshot {
            lan_enabled: self.lan_enabled(),
            bindings,
        }
    }

    pub fn set_active_bindings(&self, bindings: Vec<ListenerBinding>) {
        let mut bindings = bindings;
        bindings.sort();
        bindings.dedup();
        *self
            .inner
            .active_bindings
            .lock()
            .expect("network policy lock poisoned") = bindings;
    }
}

pub fn discover_desired_bindings(policy: &SharedNetworkPolicy) -> Vec<ListenerBinding> {
    build_desired_bindings(
        policy.port(),
        discover_tailscale_ipv4(),
        discover_private_ipv4s(),
        policy.lan_enabled(),
    )
}

pub fn build_desired_bindings(
    port: u16,
    tailscale_ipv4: Option<Ipv4Addr>,
    private_ipv4s: Vec<Ipv4Addr>,
    lan_enabled: bool,
) -> Vec<ListenerBinding> {
    let mut bindings = vec![ListenerBinding {
        kind: ListenerKind::Loopback,
        addr: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
    }];

    if let Some(ip) = tailscale_ipv4 {
        bindings.push(ListenerBinding {
            kind: ListenerKind::Tailscale,
            addr: SocketAddr::from((ip, port)),
        });
    }

    if lan_enabled {
        let mut lan_ips = private_ipv4s
            .into_iter()
            .filter(|ip| !ip.is_loopback() && !is_tailscale_ipv4(*ip))
            .collect::<Vec<_>>();
        lan_ips.sort();
        lan_ips.dedup();
        for ip in lan_ips {
            bindings.push(ListenerBinding {
                kind: ListenerKind::Lan,
                addr: SocketAddr::from((ip, port)),
            });
        }
    }

    bindings.sort();
    bindings.dedup();
    bindings
}

pub fn peer_allowed(kind: ListenerKind, peer_ip: IpAddr, lan_enabled: bool) -> bool {
    match kind {
        ListenerKind::Loopback => peer_ip.is_loopback(),
        ListenerKind::Tailscale => is_tailscale_ip(peer_ip),
        ListenerKind::Lan => lan_enabled && is_private_lan_ip(peer_ip),
    }
}

pub fn is_tailscale_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => is_tailscale_ipv4(ipv4),
        IpAddr::V6(_) => false,
    }
}

pub fn is_private_lan_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => ipv4.is_private() && !ipv4.is_loopback() && !is_tailscale_ipv4(ipv4),
        IpAddr::V6(_) => false,
    }
}

fn is_tailscale_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000
}

fn discover_tailscale_ipv4() -> Option<Ipv4Addr> {
    for command in tailscale_command_candidates() {
        if let Some(ip) = tailscale_ipv4_from_command(&command) {
            return Some(ip);
        }
    }
    None
}

fn tailscale_command_candidates() -> Vec<PathBuf> {
    let candidates = vec![PathBuf::from("tailscale")];

    #[cfg(windows)]
    {
        let mut candidates = candidates;
        for env_name in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(env_name) {
                candidates.push(PathBuf::from(base).join("Tailscale").join("tailscale.exe"));
            }
        }
        return candidates;
    }

    #[cfg(not(windows))]
    candidates
}

fn tailscale_ipv4_from_command(command: &Path) -> Option<Ipv4Addr> {
    let output = Command::new(command).args(["ip", "-4"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_tailscale_ip_output(&String::from_utf8_lossy(&output.stdout))
}

fn discover_private_ipv4s() -> Vec<Ipv4Addr> {
    #[cfg(windows)]
    {
        private_ipv4s_from_windows()
    }

    #[cfg(not(windows))]
    {
        private_ipv4s_from_unix()
    }
}

#[cfg(windows)]
fn private_ipv4s_from_windows() -> Vec<Ipv4Addr> {
    let powershell_output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "Get-NetIPAddress -AddressFamily IPv4 | Select-Object -ExpandProperty IPAddress",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_plain_ipv4_list(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();

    if !powershell_output.is_empty() {
        return powershell_output;
    }

    Command::new("ipconfig")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_ipconfig_ipv4_output(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn private_ipv4s_from_unix() -> Vec<Ipv4Addr> {
    let ip_output = Command::new("ip")
        .args(["-4", "-o", "addr", "show", "up", "scope", "global"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_linux_ip_addr_output(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();

    if !ip_output.is_empty() {
        return ip_output;
    }

    Command::new("hostname")
        .arg("-I")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_plain_ipv4_list(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

fn parse_tailscale_ip_output(text: &str) -> Option<Ipv4Addr> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(|line| line.parse::<Ipv4Addr>().ok())
}

fn parse_plain_ipv4_list(text: &str) -> Vec<Ipv4Addr> {
    let mut ips = text
        .split_whitespace()
        .filter_map(|token| token.parse::<Ipv4Addr>().ok())
        .filter(|ip| ip.is_private() && !ip.is_loopback() && !is_tailscale_ipv4(*ip))
        .collect::<Vec<_>>();
    ips.sort();
    ips.dedup();
    ips
}

fn parse_linux_ip_addr_output(text: &str) -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();

    for line in text.lines() {
        let mut parts = line.split_whitespace();
        while let Some(part) = parts.next() {
            if part == "inet" {
                if let Some(ip_with_prefix) = parts.next() {
                    if let Some(ip_text) = ip_with_prefix.split('/').next() {
                        if let Ok(ip) = ip_text.parse::<Ipv4Addr>() {
                            if ip.is_private() && !is_tailscale_ipv4(ip) {
                                ips.push(ip);
                            }
                        }
                    }
                }
                break;
            }
        }
    }

    ips.sort();
    ips.dedup();
    ips
}

#[cfg(any(windows, test))]
fn parse_ipconfig_ipv4_output(text: &str) -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();

    for line in text.lines() {
        if let Some((_, value)) = line.split_once(':') {
            let candidate = value
                .trim()
                .trim_matches(|ch| ch == '(' || ch == ')')
                .split_whitespace()
                .next()
                .unwrap_or_default();
            if let Ok(ip) = candidate.parse::<Ipv4Addr>() {
                if ip.is_private() && !ip.is_loopback() && !is_tailscale_ipv4(ip) {
                    ips.push(ip);
                }
            }
        }
    }

    ips.sort();
    ips.dedup();
    ips
}

#[cfg(test)]
mod tests {
    use super::{
        build_desired_bindings, is_private_lan_ip, is_tailscale_ip, parse_ipconfig_ipv4_output,
        parse_linux_ip_addr_output, parse_plain_ipv4_list, parse_tailscale_ip_output, peer_allowed,
        ListenerBinding, ListenerKind, SharedNetworkPolicy,
    };
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn parses_first_tailscale_ipv4_from_cli_output() {
        let output = "100.64.12.34\n";
        let ip = parse_tailscale_ip_output(output);
        assert_eq!(ip, Some(Ipv4Addr::new(100, 64, 12, 34)));
    }

    #[test]
    fn parses_linux_private_ipv4_inventory() {
        let output = "\
2: wlp3s0    inet 192.168.1.23/24 brd 192.168.1.255 scope global dynamic wlp3s0\n\
5: tailscale0    inet 100.88.2.4/32 scope global tailscale0\n\
6: enp0s31f6    inet 10.0.0.8/24 brd 10.0.0.255 scope global dynamic enp0s31f6\n";
        assert_eq!(
            parse_linux_ip_addr_output(output),
            vec![Ipv4Addr::new(10, 0, 0, 8), Ipv4Addr::new(192, 168, 1, 23)]
        );
    }

    #[test]
    fn parses_windows_private_ipv4_inventory() {
        let output = "\
IPv4 Address. . . . . . . . . . . : 192.168.0.40\n\
IPv4 Address. . . . . . . . . . . : 100.99.1.4\n\
IPv4 Address. . . . . . . . . . . : 10.10.0.4\n";
        assert_eq!(
            parse_ipconfig_ipv4_output(output),
            vec![Ipv4Addr::new(10, 10, 0, 4), Ipv4Addr::new(192, 168, 0, 40)]
        );
    }

    #[test]
    fn parses_plain_ipv4_lists() {
        let output = "192.168.1.23 100.88.2.4 10.0.0.8\n";
        assert_eq!(
            parse_plain_ipv4_list(output),
            vec![Ipv4Addr::new(10, 0, 0, 8), Ipv4Addr::new(192, 168, 1, 23)]
        );
    }

    #[test]
    fn desired_bindings_keep_loopback_only_by_default() {
        assert_eq!(
            build_desired_bindings(8080, None, vec![Ipv4Addr::new(192, 168, 1, 23)], false),
            vec![ListenerBinding {
                kind: ListenerKind::Loopback,
                addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 8080)),
            }]
        );
    }

    #[test]
    fn desired_bindings_include_tailscale_and_lan_when_enabled() {
        assert_eq!(
            build_desired_bindings(
                8080,
                Some(Ipv4Addr::new(100, 64, 12, 34)),
                vec![
                    Ipv4Addr::new(192, 168, 1, 23),
                    Ipv4Addr::new(10, 0, 0, 8),
                    Ipv4Addr::new(100, 64, 12, 34),
                ],
                true
            ),
            vec![
                ListenerBinding {
                    kind: ListenerKind::Loopback,
                    addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 8080)),
                },
                ListenerBinding {
                    kind: ListenerKind::Tailscale,
                    addr: SocketAddr::from((Ipv4Addr::new(100, 64, 12, 34), 8080)),
                },
                ListenerBinding {
                    kind: ListenerKind::Lan,
                    addr: SocketAddr::from((Ipv4Addr::new(10, 0, 0, 8), 8080)),
                },
                ListenerBinding {
                    kind: ListenerKind::Lan,
                    addr: SocketAddr::from((Ipv4Addr::new(192, 168, 1, 23), 8080)),
                },
            ]
        );
    }

    #[test]
    fn peer_acl_rejects_wrong_scope() {
        assert!(peer_allowed(
            ListenerKind::Loopback,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            false
        ));
        assert!(peer_allowed(
            ListenerKind::Tailscale,
            IpAddr::V4(Ipv4Addr::new(100, 100, 1, 2)),
            false
        ));
        assert!(peer_allowed(
            ListenerKind::Lan,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 22)),
            true
        ));
        assert!(!peer_allowed(
            ListenerKind::Lan,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 22)),
            false
        ));
        assert!(!peer_allowed(
            ListenerKind::Tailscale,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 22)),
            false
        ));
    }

    #[test]
    fn network_policy_defaults_to_lan_off_and_toggles() {
        let policy = SharedNetworkPolicy::new(8080, false);
        assert!(!policy.lan_enabled());
        assert!(policy.set_lan_enabled(true));
        assert!(policy.lan_enabled());
        assert_eq!(policy.toggle_lan(), false);
        assert!(!policy.lan_enabled());
    }

    #[test]
    fn address_classifiers_distinguish_lan_and_tailscale() {
        assert!(is_private_lan_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2))));
        assert!(!is_private_lan_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 1, 2))));
        assert!(is_tailscale_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 1, 2))));
        assert!(!is_tailscale_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2))));
    }
}
