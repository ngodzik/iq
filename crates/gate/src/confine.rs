//! The helper's process confined, before it becomes the command.
//!
//! Without the internet, a command still has localhost: on Linux it runs in
//! a network of the sandbox's own, where only the loopback device is,
//! shared by the commands of one sandbox so that a server one starts
//! answers the next; on macOS, Seatbelt lets it reach localhost only.

use std::ffi::OsString;

use crate::error::GateError;
use crate::sandbox::Policy;

/// Confines this process, and what it starts, to `policy`, and gives the
/// command to become: `program` with `args` as they are, the process being
/// held already.
#[cfg(target_os = "linux")]
pub(crate) fn confine(
    policy: &Policy,
    program: OsString,
    args: Vec<OsString>,
) -> Result<(OsString, Vec<OsString>), GateError> {
    linux::confine(policy)?;
    Ok((program, args))
}

/// Gives the command to become: `program` with `args` under Seatbelt,
/// which holds what `sandbox-exec` starts. The helper is not confined
/// itself: it only becomes `sandbox-exec`.
#[cfg(target_os = "macos")]
pub(crate) fn confine(
    policy: &Policy,
    program: OsString,
    args: Vec<OsString>,
) -> Result<(OsString, Vec<OsString>), GateError> {
    // The system's own, never one found on the `PATH`.
    let exec = std::path::Path::new("/usr/bin/sandbox-exec");
    if !exec.exists() {
        return Err(GateError::Unsupported(format!(
            "{} is not there",
            exec.display()
        )));
    }
    Ok((
        exec.as_os_str().to_owned(),
        crate::seatbelt::arguments(policy, program, args),
    ))
}

/// Refuses: no sandbox is written for this system.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn confine(
    policy: &Policy,
    program: OsString,
    args: Vec<OsString>,
) -> Result<(OsString, Vec<OsString>), GateError> {
    let _ = (policy, program, args);
    Err(GateError::Unsupported(format!(
        "iq holds no command in a sandbox on {} yet",
        std::env::consts::OS
    )))
}

/// Makes a network of its own where only localhost answers, says so on
/// its output, and holds it until its input ends, for the commands of one
/// sandbox to join. Returns only when that fails, or when its input ends.
#[cfg(target_os = "linux")]
pub(crate) fn hold_network() -> GateError {
    use std::io::{Read, Write};
    if let Err(error) = linux::own_network() {
        return error;
    }
    let mut out = std::io::stdout();
    if let Err(error) = writeln!(out, "{}", crate::sandbox::HOLDING).and_then(|()| out.flush()) {
        return GateError::Spawn(error);
    }
    // Held as long as the sandbox keeps its end of the input open.
    let _ = std::io::stdin().read_to_end(&mut Vec::new());
    GateError::Confine("the sandbox let its network go".to_owned())
}

/// Refuses: on this system, commands reach localhost through the host's
/// own network, as Seatbelt allows.
#[cfg(not(target_os = "linux"))]
pub(crate) fn hold_network() -> GateError {
    GateError::Unsupported("a network of the sandbox's own is made on Linux only".to_owned())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::os::fd::AsRawFd;

    use landlock::{
        ABI, Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
        Scope, path_beneath_rules,
    };
    use nix::sched::CloneFlags;
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule,
    };

    use super::{GateError, Policy};

    /// The newest Landlock the rules are written for. An older kernel
    /// enforces what it knows; one that knows none refuses to run.
    const ABI_WANTED: ABI = ABI::V6;

    fn failed(error: impl std::fmt::Display) -> GateError {
        GateError::Confine(error.to_string())
    }

    pub(super) fn confine(policy: &Policy) -> Result<(), GateError> {
        // Without the internet, a network of the sandbox's own, where only
        // localhost answers. Where none can be made, no socket reaches out
        // at all, localhost included.
        let apart = !policy.network
            && match policy.shared {
                Some(holder) => join_network(holder).or_else(|_| own_network()),
                None => own_network(),
            }
            .is_ok();
        let mut ruleset = Ruleset::default()
            .handle_access(AccessFs::from_all(ABI_WANTED))
            .map_err(failed)?
            // Signals and abstract sockets kept within the sandbox: no
            // stopping iq, no reaching the desktop's buses.
            .scope(Scope::from_all(ABI_WANTED))
            .map_err(failed)?;
        if !policy.network && !apart {
            ruleset = ruleset
                .handle_access(AccessNet::from_all(ABI_WANTED))
                .map_err(failed)?;
        }
        let status = ruleset
            .create()
            .map_err(failed)?
            .add_rules(path_beneath_rules(
                &policy.read,
                AccessFs::from_read(ABI_WANTED),
            ))
            .map_err(failed)?
            .add_rules(path_beneath_rules(
                &policy.write,
                AccessFs::from_all(ABI_WANTED),
            ))
            .map_err(failed)?
            .restrict_self()
            .map_err(failed)?;
        if status.ruleset == RulesetStatus::NotEnforced {
            return Err(GateError::Unsupported(
                "this kernel does not enforce Landlock".to_owned(),
            ));
        }
        sockets(policy.network || apart)
    }

    /// Moves this process into a network of its own, with a user namespace
    /// to have the right to: the person's own user and group within it,
    /// nothing else, and the loopback device up.
    pub(super) fn own_network() -> Result<(), GateError> {
        let user = nix::unistd::getuid();
        let group = nix::unistd::getgid();
        nix::sched::unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNET)
            .map_err(failed)?;
        std::fs::write("/proc/self/setgroups", "deny").map_err(failed)?;
        std::fs::write("/proc/self/uid_map", format!("{user} {user} 1")).map_err(failed)?;
        std::fs::write("/proc/self/gid_map", format!("{group} {group} 1")).map_err(failed)?;
        loopback_up()
    }

    /// Moves this process into the network the process `holder` holds.
    fn join_network(holder: u32) -> Result<(), GateError> {
        let open = |kind: &str| std::fs::File::open(format!("/proc/{holder}/ns/{kind}"));
        let user = open("user").map_err(failed)?;
        let network = open("net").map_err(failed)?;
        nix::sched::setns(&user, CloneFlags::CLONE_NEWUSER).map_err(failed)?;
        nix::sched::setns(&network, CloneFlags::CLONE_NEWNET).map_err(failed)
    }

    /// Brings the loopback device up, as `ip link set lo up` does: one
    /// request to the kernel over netlink, and its acknowledgement.
    fn loopback_up() -> Result<(), GateError> {
        use nix::sys::socket::{
            AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, recv, sendto,
            socket,
        };
        let socket = socket(
            AddressFamily::Netlink,
            SockType::Raw,
            SockFlag::SOCK_CLOEXEC,
            SockProtocol::NetlinkRoute,
        )
        .map_err(failed)?;
        let up = libc::IFF_UP.unsigned_abs();
        let flags = u16::try_from(libc::NLM_F_REQUEST | libc::NLM_F_ACK).map_err(failed)?;
        let mut request = Vec::with_capacity(32);
        // The message's header: its length, kind, flags, number, sender.
        request.extend_from_slice(&32u32.to_ne_bytes());
        request.extend_from_slice(&libc::RTM_NEWLINK.to_ne_bytes());
        request.extend_from_slice(&flags.to_ne_bytes());
        request.extend_from_slice(&1u32.to_ne_bytes());
        request.extend_from_slice(&0u32.to_ne_bytes());
        // The device: any family, its index (the loopback is the first
        // device of a network), the flag set and the flag changed.
        request.extend_from_slice(&[0, 0]);
        request.extend_from_slice(&0u16.to_ne_bytes());
        request.extend_from_slice(&1i32.to_ne_bytes());
        request.extend_from_slice(&up.to_ne_bytes());
        request.extend_from_slice(&up.to_ne_bytes());
        sendto(
            socket.as_raw_fd(),
            &request,
            &NetlinkAddr::new(0, 0),
            MsgFlags::empty(),
        )
        .map_err(failed)?;
        let mut answer = [0u8; 256];
        let read = recv(socket.as_raw_fd(), &mut answer, MsgFlags::empty()).map_err(failed)?;
        let kind = answer.get(4..6).map(|b| u16::from_ne_bytes([b[0], b[1]]));
        let error = answer
            .get(16..20)
            .filter(|_| read >= 20)
            .map(|b| i32::from_ne_bytes([b[0], b[1], b[2], b[3]]));
        match (kind, error) {
            (Some(kind), Some(0)) if i32::from(kind) == libc::NLMSG_ERROR => Ok(()),
            (_, Some(code)) => Err(failed(format!(
                "the loopback device could not be brought up: {}",
                std::io::Error::from_raw_os_error(-code)
            ))),
            _ => Err(failed(
                "the kernel's answer on the loopback device was not read",
            )),
        }
    }

    /// No io_uring, which opens sockets without asking for them; and,
    /// unless `reaching` (the internet given, or a network of the
    /// sandbox's own), no socket but Unix ones.
    fn sockets(reaching: bool) -> Result<(), GateError> {
        let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        for call in [
            libc::SYS_io_uring_setup,
            libc::SYS_io_uring_enter,
            libc::SYS_io_uring_register,
        ] {
            // No condition: refused whatever its arguments.
            rules.insert(call, Vec::new());
        }
        if !reaching {
            let not_unix = SeccompCondition::new(
                0,
                SeccompCmpArgLen::Dword,
                SeccompCmpOp::Ne,
                u64::from(libc::AF_UNIX.unsigned_abs()),
            )
            .map_err(failed)?;
            rules.insert(
                libc::SYS_socket,
                vec![SeccompRule::new(vec![not_unix]).map_err(failed)?],
            );
        }
        let arch = std::env::consts::ARCH.try_into().map_err(failed)?;
        let filter = SeccompFilter::new(
            rules,
            SeccompAction::Allow,
            SeccompAction::Errno(libc::EPERM.unsigned_abs()),
            arch,
        )
        .map_err(failed)?;
        let program: BpfProgram = filter.try_into().map_err(failed)?;
        seccompiler::apply_filter(&program).map_err(failed)
    }
}
