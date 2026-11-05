// SPDX-License-Identifier: AGPL-3.0-only
//! Reaching a host over TCP, address by address: a network can hand out IPv6 and then drop it, and
//! a limit around all of a host's addresses would be used up by the first one. Like Android
//! (`CspSocket`, `ChatServerAddressProviderImpl`): IPv6 first, 3 s for each IPv6 address and 15 s
//! for each IPv4 address.

use std::{
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use tokio::{
    net::{TcpStream, lookup_host},
    time::timeout,
};

const IPV6_LIMIT: Duration = Duration::from_secs(3);
const IPV4_LIMIT: Duration = Duration::from_secs(15);
const LOOKUP_LIMIT: Duration = Duration::from_secs(15);

const fn limit_for(address: SocketAddr) -> Duration {
    match address.ip() {
        IpAddr::V6(_) => IPV6_LIMIT,
        IpAddr::V4(_) => IPV4_LIMIT,
    }
}

/// The first of these that connects, each within its own limit. Otherwise what the last one failed
/// with.
async fn first_to_connect<T, Connect, Attempt>(
    mut addresses: Vec<SocketAddr>,
    mut last_error: Option<io::Error>,
    connect: Connect,
) -> io::Result<T>
where
    Connect: Fn(SocketAddr) -> Attempt,
    Attempt: Future<Output = io::Result<T>>,
{
    // Stable: within a family the order stays what the resolver prefers.
    addresses.sort_by_key(|address| address.is_ipv4());
    for address in addresses {
        match timeout(limit_for(address), connect(address)).await {
            Ok(Ok(connected)) => return Ok(connected),
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => last_error = Some(io::ErrorKind::TimedOut.into()),
        }
    }
    Err(last_error.unwrap_or_else(|| io::ErrorKind::AddrNotAvailable.into()))
}

/// Connect to the first address that answers, of all the addresses of these hosts and ports. A host
/// that can't be looked up leaves the others.
pub(crate) async fn connect_tcp(endpoints: &[(&str, u16)]) -> io::Result<TcpStream> {
    let mut addresses = Vec::new();
    let mut last_error = None;
    for endpoint in endpoints {
        match timeout(LOOKUP_LIMIT, lookup_host(*endpoint)).await {
            Ok(Ok(found)) => addresses.extend(found),
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => last_error = Some(io::ErrorKind::TimedOut.into()),
        }
    }
    first_to_connect(addresses, last_error, TcpStream::connect).await
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use tokio::{net::TcpListener, time::Instant};

    use super::*;

    fn v4() -> SocketAddr {
        "192.0.2.1:5222".parse().unwrap()
    }

    fn v6(last: u16) -> SocketAddr {
        SocketAddr::new(format!("2001:db8::{last:x}").parse().unwrap(), 5222)
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn dropped_ipv6_leaves_time_for_ipv4() {
        let start = Instant::now();
        let connected = first_to_connect(vec![v4(), v6(1), v6(2)], None, |address| async move {
            if address.is_ipv6() {
                std::future::pending::<()>().await;
            }
            Ok(address)
        })
        .await;
        assert_eq!(connected.unwrap(), v4());
        assert_eq!(start.elapsed(), IPV6_LIMIT * 2, "3 s per IPv6 address");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ipv4_gets_its_own_longer_limit() {
        let start = Instant::now();
        let connected = first_to_connect::<(), _, _>(vec![v4()], None, |_| async {
            std::future::pending().await
        })
        .await;
        assert_eq!(connected.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(start.elapsed(), IPV4_LIMIT);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ipv6_goes_first_and_a_refusal_moves_on() {
        let tried = RefCell::new(Vec::new());
        let connected = first_to_connect(vec![v4(), v6(1)], None, |address| {
            tried.borrow_mut().push(address);
            async move {
                if address.is_ipv6() {
                    Err(io::ErrorKind::ConnectionRefused.into())
                } else {
                    Ok(address)
                }
            }
        })
        .await;
        assert_eq!(connected.unwrap(), v4());
        assert_eq!(*tried.borrow(), [v6(1), v4()]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_address_says_why() {
        let lookup_failed = io::Error::from(io::ErrorKind::NotFound);
        let connected =
            first_to_connect::<(), _, _>(Vec::new(), Some(lookup_failed), |_| async { Ok(()) })
                .await;
        assert_eq!(connected.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connects_to_a_real_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stream = connect_tcp(&[("127.0.0.1", port)]).await.unwrap();
        assert_eq!(stream.peer_addr().unwrap().port(), port);
    }
}
