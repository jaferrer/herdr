//! `herdr connection` — manage which server this client attaches to.
//!
//! Purely client-local: it edits the saved Local/SSH profiles and the active
//! selection, and never talks to a server. Switching destinations therefore
//! cannot stop anything; the panes on the machine you leave keep running.

use crate::remote::profiles::RemoteProfiles;

pub(super) fn run_connection_command(args: &[String]) -> std::io::Result<i32> {
    match args.first().map(|arg| arg.as_str()) {
        Some("list") if args.len() == 1 => list(),
        Some("add") => add(&args[1..]),
        Some("use") => use_profile(&args[1..]),
        Some("rename") => rename(&args[1..]),
        Some("remove") => remove(&args[1..]),
        Some("help" | "--help" | "-h") => {
            print_help();
            Ok(0)
        }
        _ => {
            print_help();
            Ok(2)
        }
    }
}

fn print_help() {
    println!("Usage: herdr connection <subcommand>");
    println!();
    println!("  list                      Show saved destinations and the active one");
    println!("  add <name> <ssh-target>   Save an SSH destination and make it active");
    println!("  use <name|local>          Attach to this destination from now on");
    println!("  rename <name> <new-name>  Rename a saved destination");
    println!("  remove <name>             Forget a saved destination");
    println!();
    println!("The active destination is what a bare `herdr` attaches to.");
    println!("`local` is the server on this machine. `--remote` still overrides it.");
    println!("Passwords are never stored: authentication is plain OpenSSH.");
}

fn load() -> RemoteProfiles {
    RemoteProfiles::load()
}

fn save(profiles: &RemoteProfiles) -> std::io::Result<i32> {
    match profiles.save_to(&crate::remote::profiles::profiles_path()) {
        Ok(()) => Ok(0),
        Err(err) => {
            eprintln!("error: could not save connection profiles: {err}");
            Ok(1)
        }
    }
}

fn list() -> std::io::Result<i32> {
    let profiles = load();
    let active_is_local = profiles.active_target().is_none();
    println!("{} local", if active_is_local { "*" } else { " " });
    for profile in &profiles.profiles {
        let active = profiles.active.as_deref() == Some(profile.name.as_str());
        println!(
            "{} {}  {}",
            if active { "*" } else { " " },
            profile.name,
            profile.target
        );
    }
    Ok(0)
}

fn add(args: &[String]) -> std::io::Result<i32> {
    let [name, target] = args else {
        eprintln!("usage: herdr connection add <name> <ssh-target>");
        return Ok(2);
    };

    let mut profiles = load();
    if let Err(err) = profiles.upsert(name, target) {
        eprintln!("error: {err}");
        return Ok(2);
    }
    let code = save(&profiles)?;
    if code == 0 {
        println!("saved '{name}' and made it active");
    }
    Ok(code)
}

fn use_profile(args: &[String]) -> std::io::Result<i32> {
    let [name] = args else {
        eprintln!("usage: herdr connection use <name|local>");
        return Ok(2);
    };

    let mut profiles = load();
    let selection = (name != "local").then_some(name.as_str());
    if let Err(err) = profiles.set_active(selection) {
        eprintln!("error: {err}");
        return Ok(2);
    }
    let code = save(&profiles)?;
    if code == 0 {
        println!("attaching to '{name}' from now on");
    }
    Ok(code)
}

fn rename(args: &[String]) -> std::io::Result<i32> {
    let [current, new_name] = args else {
        eprintln!("usage: herdr connection rename <name> <new-name>");
        return Ok(2);
    };

    let mut profiles = load();
    if let Err(err) = profiles.rename(current, new_name) {
        eprintln!("error: {err}");
        return Ok(2);
    }
    let code = save(&profiles)?;
    if code == 0 {
        println!("renamed '{current}' to '{new_name}'");
    }
    Ok(code)
}

fn remove(args: &[String]) -> std::io::Result<i32> {
    let [name] = args else {
        eprintln!("usage: herdr connection remove <name>");
        return Ok(2);
    };

    let mut profiles = load();
    if let Err(err) = profiles.remove(name) {
        eprintln!("error: {err}");
        return Ok(2);
    }
    let code = save(&profiles)?;
    if code == 0 {
        println!("removed '{name}'");
    }
    Ok(code)
}
