use rtools::cli::ShellCommand;
use rtools::instance::InstanceName;
use std::thread;

fn unique(tag: &str) -> InstanceName {
    let name = format!("rtools-test-{}-{tag}.sock", std::process::id());
    // Linux uses socket files, other platforms namespaced names.
    if cfg!(target_os = "linux") {
        InstanceName(std::env::temp_dir().join(name).display().to_string())
    } else {
        InstanceName(name)
    }
}

/// CLI commands retain their wire spelling, including an idempotent background start.
#[test]
fn commands_parse_and_display() {
    assert_eq!(
        " Toggle ".parse::<ShellCommand>().unwrap(),
        ShellCommand::Toggle
    );
    assert_eq!("quit".parse::<ShellCommand>().unwrap().to_string(), "quit");
    assert_eq!(
        "settings".parse::<ShellCommand>().unwrap(),
        ShellCommand::Settings
    );
    assert_eq!(ShellCommand::Settings.to_string(), "settings");
    assert_eq!(
        "background".parse::<ShellCommand>().unwrap(),
        ShellCommand::Background
    );
    assert_eq!(ShellCommand::Background.to_string(), "background");
    assert!(
        "open"
            .parse::<ShellCommand>()
            .unwrap_err()
            .to_string()
            .contains("toggle")
    );
}

#[test]
fn send_without_server_fails_fast() {
    assert!(unique("absent").send(ShellCommand::Toggle).is_err());
}

#[test]
fn server_round_trip() {
    let name = unique("roundtrip");
    let server = name.bind().unwrap();
    let reader = thread::spawn(move || server.incoming().take(2).collect::<Vec<_>>());
    name.send(ShellCommand::Toggle).unwrap();
    name.send(ShellCommand::Quit).unwrap();
    let got: Vec<_> = reader
        .join()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(got, [ShellCommand::Toggle, ShellCommand::Quit]);
}

#[cfg(target_os = "linux")]
#[test]
fn silent_client_does_not_block_later_commands() {
    let name = unique("silent");
    let server = name.bind().unwrap();
    let reader = thread::spawn(move || server.incoming().take(2).collect::<Vec<_>>());
    // Connects but never writes a command line.
    let _silent = std::os::unix::net::UnixStream::connect(&name.0).unwrap();
    name.send(ShellCommand::Toggle).unwrap();
    let got = reader.join().unwrap();
    assert!(got[0].is_err(), "{got:?}");
    assert_eq!(*got[1].as_ref().unwrap(), ShellCommand::Toggle);
}

#[test]
fn a_live_instance_keeps_its_socket() {
    let name = unique("live");
    let server = name.bind().unwrap();
    assert!(
        name.bind().is_err(),
        "a second bind must not steal the socket"
    );
    let reader = thread::spawn(move || server.incoming().next());
    name.send(ShellCommand::Quit).unwrap();
    assert_eq!(reader.join().unwrap().unwrap().unwrap(), ShellCommand::Quit);
}

#[cfg(target_os = "linux")]
#[test]
fn a_stale_socket_is_replaced() {
    let name = unique("stale");
    let _ = std::fs::remove_file(&name.0);
    // A socket file nobody listens on, as left behind by a crashed instance.
    drop(std::os::unix::net::UnixListener::bind(&name.0).unwrap());
    let server = name.bind().unwrap();
    let reader = thread::spawn(move || server.incoming().next());
    name.send(ShellCommand::Toggle).unwrap();
    assert_eq!(
        reader.join().unwrap().unwrap().unwrap(),
        ShellCommand::Toggle
    );
}

#[cfg(target_os = "linux")]
#[test]
fn an_oversized_command_line_is_rejected() {
    use std::io::Write as _;
    let name = unique("oversized");
    let server = name.bind().unwrap();
    let reader = thread::spawn(move || server.incoming().next());
    // A valid command behind far more padding than any command needs.
    let mut client = std::os::unix::net::UnixStream::connect(&name.0).unwrap();
    let line = format!("{}toggle\n", " ".repeat(100_000));
    // The server stops reading early, so the rest of the write may fail.
    let _ = client.write_all(line.as_bytes());
    drop(client);
    assert!(reader.join().unwrap().unwrap().is_err());
}
