//! The sandbox's helper on its own, for the gate's tests: the iq command is
//! the helper otherwise, with `--iq-sandbox`.

#![deny(unsafe_code)]

fn main() {
    let mut args = std::env::args_os().skip(1).peekable();
    let error = if args.peek().is_some_and(|arg| arg == iq_gate::HOLD_FLAG) {
        iq_gate::hold_network()
    } else {
        iq_gate::sandboxed(args)
    };
    eprintln!("{}", iq_gate::helper_failure(&error));
    std::process::exit(125);
}
