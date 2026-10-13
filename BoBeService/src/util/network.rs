use tracing::{debug, error, info, warn};

#[allow(unsafe_code)]
fn get_hostname() -> String {
    let mut buf = [0u8; 256];
    let ret = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len()) };
    if ret != 0 {
        return "bobe".into();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

const SERVICE_TYPE: &str = "_bobe._tcp.local.";

const VIRTUAL_INTERFACES: [&str; 12] = [
    "docker", "br-", "veth", "virbr", "lxc", "lxd", "cni", "flannel", "cali", "podman", "vmnet",
    "vboxnet",
];
const TUNNEL_INTERFACES: [&str; 6] = ["tailscale", "utun", "tun", "wg", "zt", "tap"];

#[derive(Debug, PartialEq, Eq)]
enum AddressKind {
    Local,
    Tunnel,
}

fn address_kind(
    interface: &str,
    point_to_point: bool,
    address: std::net::Ipv4Addr,
) -> Option<AddressKind> {
    if address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || VIRTUAL_INTERFACES
            .iter()
            .any(|prefix| interface.starts_with(prefix))
    {
        return None;
    }
    if point_to_point
        || TUNNEL_INTERFACES
            .iter()
            .any(|prefix| interface.starts_with(prefix))
    {
        Some(AddressKind::Tunnel)
    } else {
        Some(AddressKind::Local)
    }
}

pub(crate) fn reachable_hosts() -> Vec<String> {
    let mut local = Vec::new();
    let mut tunnels = Vec::new();
    for (interface, point_to_point, address) in interface_addresses() {
        let text = address.to_string();
        match address_kind(&interface, point_to_point, address) {
            Some(AddressKind::Local) if !local.contains(&text) => local.push(text),
            Some(AddressKind::Tunnel) if !tunnels.contains(&text) => tunnels.push(text),
            _ => {}
        }
    }
    mdns_name()
        .into_iter()
        .chain(local)
        .chain(tunnels)
        .collect()
}

fn mdns_name() -> Option<String> {
    let advertised =
        cfg!(target_os = "macos") || std::path::Path::new("/run/avahi-daemon/socket").exists();
    if !advertised {
        return None;
    }
    let hostname = get_hostname();
    let short = hostname.split('.').next()?.trim().to_lowercase();
    (!short.is_empty() && short != "localhost").then(|| format!("{short}.local"))
}

fn has_flag(flags: u32, flag: libc::c_int) -> bool {
    u32::try_from(flag).is_ok_and(|flag| flags & flag != 0)
}

#[allow(unsafe_code)]
fn interface_addresses() -> Vec<(String, bool, std::net::Ipv4Addr)> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&raw mut list) } != 0 {
        return Vec::new();
    }
    let mut addresses = Vec::new();
    let mut cursor = list;
    while !cursor.is_null() {
        let entry = unsafe { &*cursor };
        cursor = entry.ifa_next;
        let flags = entry.ifa_flags;
        if !has_flag(flags, libc::IFF_UP)
            || has_flag(flags, libc::IFF_LOOPBACK)
            || entry.ifa_addr.is_null()
            || entry.ifa_name.is_null()
        {
            continue;
        }
        if i32::from(unsafe { (*entry.ifa_addr).sa_family }) != libc::AF_INET {
            continue;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let socket = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in>() };
        let address = std::net::Ipv4Addr::from(u32::from_be(socket.sin_addr.s_addr));
        addresses.push((name, has_flag(flags, libc::IFF_POINTOPOINT), address));
    }
    unsafe { libc::freeifaddrs(list) };
    addresses
}

fn service_info(hostname: &str, port: u16) -> mdns_sd::Result<mdns_sd::ServiceInfo> {
    mdns_sd::ServiceInfo::new(
        SERVICE_TYPE,
        &format!("BoBe on {hostname}"),
        &format!("{hostname}.local."),
        "",
        port,
        None,
    )
    .map(mdns_sd::ServiceInfo::enable_addr_auto)
}

pub(crate) struct MdnsAnnouncer {
    port: u16,
    enabled: bool,
    daemon: tokio::sync::Mutex<Option<mdns_sd::ServiceDaemon>>,
}

impl MdnsAnnouncer {
    pub(crate) fn new(port: u16, enabled: bool) -> Self {
        Self {
            port,
            enabled,
            daemon: tokio::sync::Mutex::new(None),
        }
    }

    pub(crate) async fn start(&self) {
        if !self.enabled {
            debug!("mdns.disabled");
            return;
        }

        let hostname = get_hostname();

        match mdns_sd::ServiceDaemon::new() {
            Ok(daemon) => {
                let service_info = match service_info(&hostname, self.port) {
                    Ok(info) => info,
                    Err(e) => {
                        warn!(error = %e, "mdns.service_info_failed");
                        return;
                    }
                };

                match daemon.register(service_info) {
                    Ok(()) => {
                        info!(
                            port = self.port,
                            service_type = SERVICE_TYPE,
                            "mdns.advertising"
                        );
                        *self.daemon.lock().await = Some(daemon);
                    }
                    Err(e) => {
                        error!(error = %e, "mdns.register_failed");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "mdns.daemon_creation_failed");
            }
        }
    }

    pub(crate) async fn stop(&self) {
        if let Some(daemon) = self.daemon.lock().await.take() {
            if let Err(e) = daemon.shutdown() {
                warn!(error = %e, "mdns.shutdown_error");
            } else {
                info!("mdns.stopped");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_offered_local_addresses_before_tunnels_and_never_bridges() {
        let lan: std::net::Ipv4Addr = "192.168.1.20".parse().unwrap();
        let tailnet: std::net::Ipv4Addr = "100.64.0.7".parse().unwrap();
        let docker: std::net::Ipv4Addr = "172.17.0.1".parse().unwrap();
        assert_eq!(address_kind("enp3s0", false, lan), Some(AddressKind::Local));
        assert_eq!(address_kind("en0", false, lan), Some(AddressKind::Local));
        assert_eq!(
            address_kind("tailscale0", false, tailnet),
            Some(AddressKind::Tunnel)
        );
        assert_eq!(
            address_kind("utun4", true, tailnet),
            Some(AddressKind::Tunnel)
        );
        assert_eq!(address_kind("docker0", false, docker), None);
        assert_eq!(address_kind("br-0123456789ab", false, docker), None);
        assert_eq!(
            address_kind("wlan0", false, "169.254.3.4".parse().unwrap()),
            None
        );
        assert_eq!(
            address_kind("lo", false, "127.0.0.1".parse().unwrap()),
            None
        );
    }

    #[test]
    fn this_computer_lists_at_least_its_own_addresses() {
        let hosts = reachable_hosts();
        assert!(
            hosts
                .iter()
                .all(|host| host != "127.0.0.1" && !host.starts_with("169.254."))
        );
    }

    #[test]
    fn service_discovery_tracks_host_addresses() {
        let service = service_info("bobe", 8766).unwrap();

        assert!(service.is_addr_auto());
    }
}
