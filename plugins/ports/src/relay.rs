//! The usual ports of localhost led to the worktree served: each
//! connection to one of them is passed to the port of the worktree's own
//! its stack publishes it on, both ways, until either end closes. Leading
//! them elsewhere is instant: what was open stays where it was, what opens
//! next goes to the other worktree.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The usual ports, each led to a worktree's own.
#[derive(Debug, Default)]
pub struct Relay {
    /// Where each usual port leads.
    routes: Arc<RwLock<HashMap<u16, u16>>>,
    /// What listens on each, on IPv4 and IPv6.
    listening: HashMap<u16, Vec<JoinHandle<()>>>,
}

impl Relay {
    /// Leads the usual ports of `routes` to the worktree's own from now
    /// on: listens on those not listened on yet, and lets go of those no
    /// longer led. Gives the ports it could not listen on, with why: held
    /// by another program, as a stack started outside the plugin would
    /// hold them.
    pub async fn lead(&mut self, routes: HashMap<u16, u16>) -> Vec<(u16, std::io::Error)> {
        if let Ok(mut led) = self.routes.write() {
            led.clone_from(&routes);
        }
        let gone: Vec<u16> = self
            .listening
            .keys()
            .filter(|usual| !routes.contains_key(usual))
            .copied()
            .collect();
        for usual in gone {
            if let Some(tasks) = self.listening.remove(&usual) {
                for task in tasks {
                    task.abort();
                }
            }
        }
        let mut refused = Vec::new();
        let mut usual: Vec<u16> = routes.keys().copied().collect();
        usual.sort_unstable();
        for port in usual {
            if self.listening.contains_key(&port) {
                continue;
            }
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => {
                    let mut tasks = vec![tokio::spawn(listen(
                        listener,
                        port,
                        Arc::clone(&self.routes),
                    ))];
                    // A browser may try localhost's IPv6 first.
                    if let Ok(six) = TcpListener::bind(("::1", port)).await {
                        tasks.push(tokio::spawn(listen(six, port, Arc::clone(&self.routes))));
                    }
                    self.listening.insert(port, tasks);
                }
                Err(error) => refused.push((port, error)),
            }
        }
        refused
    }

    /// The usual ports led, in order.
    #[must_use]
    pub fn led(&self) -> Vec<u16> {
        let mut led: Vec<u16> = self.listening.keys().copied().collect();
        led.sort_unstable();
        led
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        for task in self.listening.values().flatten() {
            task.abort();
        }
    }
}

/// Takes each connection to `usual` and passes it to where it leads now.
async fn listen(listener: TcpListener, usual: u16, routes: Arc<RwLock<HashMap<u16, u16>>>) {
    loop {
        let mut inbound = match listener.accept().await {
            Ok((inbound, _)) => inbound,
            Err(_) => {
                // Out of files for a moment, say: it takes the next.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let hidden = routes
            .read()
            .ok()
            .and_then(|routes| routes.get(&usual).copied());
        let Some(hidden) = hidden else {
            continue;
        };
        tokio::spawn(async move {
            if let Ok(mut outbound) = TcpStream::connect(("127.0.0.1", hidden)).await {
                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A server on a port of its own that answers each connection with
    /// `name` and what it was sent.
    async fn server(name: &'static str) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut asked = [0u8; 4];
                    if stream.read_exact(&mut asked).await.is_ok() {
                        let said = format!("{name}:{}", String::from_utf8_lossy(&asked));
                        let _ = stream.write_all(said.as_bytes()).await;
                    }
                });
            }
        });
        port
    }

    async fn ask(port: u16) -> std::io::Result<String> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(b"ping").await?;
        let mut said = String::new();
        stream.read_to_string(&mut said).await?;
        Ok(said)
    }

    /// A port nothing listens on.
    fn unused() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[tokio::test]
    async fn a_usual_port_leads_to_the_worktree_served_and_moves_at_once() {
        let (lead, fix) = (server("lead").await, server("fix").await);
        let usual = unused();
        let mut relay = Relay::default();
        assert!(relay.lead(HashMap::from([(usual, lead)])).await.is_empty());
        assert_eq!(ask(usual).await.unwrap(), "lead:ping");
        // Held open, a connection stays where it was; the next goes to
        // the other worktree.
        let mut open = TcpStream::connect(("127.0.0.1", usual)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(relay.lead(HashMap::from([(usual, fix)])).await.is_empty());
        assert_eq!(ask(usual).await.unwrap(), "fix:ping");
        open.write_all(b"ping").await.unwrap();
        let mut said = String::new();
        open.read_to_string(&mut said).await.unwrap();
        assert_eq!(said, "lead:ping");
        assert_eq!(relay.led(), [usual]);
        // Led nowhere, it is let go.
        relay.lead(HashMap::new()).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(ask(usual).await.is_err());
        assert!(relay.led().is_empty());
    }

    #[tokio::test]
    async fn a_usual_port_another_program_holds_is_said() {
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let usual = held.local_addr().unwrap().port();
        let mut relay = Relay::default();
        let refused = relay.lead(HashMap::from([(usual, 1)])).await;
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].0, usual);
        assert!(relay.led().is_empty());
    }
}
