//! The command line tool itself, native only.

use std::io::{BufRead, Read, Write};

use anarchy::*;
use anyhow::{Context, bail};

const HELP: &str = "\
anarchy: inspect and edit a running anarchy world through its socket

usage: anarchy [-a ADDRESS] [COMMAND [ARGS...]]
       with no command, starts an interactive shell

ADDRESS is host:port or unix:<path>, default $ANARCHY_SOCKET or 127.0.0.1:7878.
TARGET is `<entity> <Type>` for a component or `@<Type>` for a resource.
Values use the scene format, such as `55`, `\"text\"` or `{ x: 1.0, tags: [\"a\"] }`.

commands:
  entities, ls                  list entities
  show <entity>                 an entity's components and their values
  resources, res                list resources and their values
  types                         list registered types
  info <Type>                   a type's fields, docs and hints, as JSON
  get TARGET [path]             read a value, or the field at path
  set TARGET <path> <value>     set the field at path, `.` for the whole value
  insert TARGET [value]         add or replace a component or resource
  remove TARGET                 remove a component or resource
  spawn <file | ->              spawn a text scene from a file or stdin
  despawn <entity> [-r]         remove an entity, with -r also its descendants
  parent <child> <parent|none>  set or clear an entity's parent
  save [entity...]              print entities as a text scene, all roots by default
  help                          show this
  quit, exit                    leave the shell";

pub fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut address = std::env::var("ANARCHY_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET_ADDRESS.into());
    if let Some(i) = args.iter().position(|a| a == "-a" || a == "--address") {
        address = args[i + 1..].first().context("-a needs an address")?.to_owned();
        args.drain(i..i + 2);
    }
    if matches!(args.first().map(String::as_str), Some("help" | "-h" | "--help")) {
        println!("{HELP}");
        return Ok(())
    }

    let mut client = SocketClient::connect(&address)?;
    if !args.is_empty() { return command(&mut client, &args.join(" ")) }

    println!("connected to {address}, type `help` for commands");
    let stdin = std::io::stdin();
    loop {
        print!("> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 { return Ok(()) }
        match line.trim() {
            "" => continue,
            "quit" | "exit" => return Ok(()),
            line => if let Err(e) = command(&mut client, line) { eprintln!("error: {e:#}") }
        }
    }
}

/// Runs one command line.
fn command(client: &mut SocketClient, line: &str) -> anyhow::Result<()> {
    let mut words = Words(line.trim());
    let name = words.next().unwrap_or_default();
    match name {
        "help" => println!("{HELP}"),
        "entities" | "ls" => {
            let Response::Entities(entities) = client.request(Request::Entities)? else { bail!("unexpected response") };
            for entity in entities {
                let name = entity.name.map(|name| format!(" #{name}")).unwrap_or_default();
                let parent = entity.parent.map(|parent| format!(" (child of {parent})")).unwrap_or_default();
                println!("{}{name}{parent}: {}", entity.id, entity.components.join(", "));
            }
        },
        "show" => {
            let entity = entity(words.next())?;
            let Response::Entity(details) = client.request(Request::Entity { entity })? else { bail!("unexpected response") };
            let mut header = details.id.to_string();
            if let Some(name) = details.name { header += &format!(" #{name}") }
            if let Some(parent) = details.parent { header += &format!(", child of {parent}") }
            if !details.children.is_empty() { header += &format!(", children {:?}", details.children) }
            println!("{header}");
            for component in details.components { println!("  {}", reflected(&component)) }
        },
        "resources" | "res" => {
            let Response::Resources(resources) = client.request(Request::Resources)? else { bail!("unexpected response") };
            for resource in resources { println!("@{}", reflected(&resource)) }
        },
        "types" => {
            let Response::Types(types) = client.request(Request::Types)? else { bail!("unexpected response") };
            for info in types {
                let kind = match (info.component, info.resource) {
                    (true, true) => "component, resource",
                    (true, false) => "component",
                    (false, true) => "resource",
                    (false, false) => "type"
                };
                println!("{} ({kind}): {}", info.name, info.type_name);
            }
        },
        "info" => {
            let type_name = words.next().context("info needs a type")?.to_string();
            let Response::TypeInfo(info) = client.request(Request::TypeInfo { type_name })? else { bail!("unexpected response") };
            println!("{}", serde_json::to_string_pretty(&info)?);
        },
        "get" => {
            let target = target(&mut words)?;
            let path = path(words.next());
            let Response::Value(value) = client.request(Request::Get { target, path })? else { bail!("unexpected response") };
            println!("{value}");
        },
        "set" => {
            let target = target(&mut words)?;
            let path = path(Some(words.next().context("set needs a path and a value")?));
            let value = parse_value(words.rest()).context("parsing the value")?;
            client.request(Request::Set { target, path, value })?;
        },
        "insert" => {
            let target = target(&mut words)?;
            let value = if words.rest().is_empty() { Value::Unit } else { parse_value(words.rest()).context("parsing the value")? };
            client.request(Request::Insert { target, value })?;
        },
        "remove" => {
            let target = target(&mut words)?;
            client.request(Request::Remove { target })?;
        },
        "spawn" => {
            let scene = match words.next().context("spawn needs a file, or - for stdin")? {
                "-" => { let mut text = String::new(); std::io::stdin().read_to_string(&mut text)?; text },
                file => std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?
            };
            let Response::Spawned(roots) = client.request(Request::Spawn { scene })? else { bail!("unexpected response") };
            println!("spawned {}", roots.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "));
        },
        "despawn" => {
            let entity = entity(words.next())?;
            let recursive = matches!(words.next(), Some("-r" | "--recursive"));
            client.request(Request::Despawn { entity, recursive })?;
        },
        "parent" => {
            let child = entity(words.next())?;
            let parent = match words.next().context("parent needs a parent, or none")? {
                "none" => None,
                parent => Some(entity(Some(parent))?)
            };
            client.request(Request::SetParent { child, parent })?;
        },
        "save" => {
            let roots = std::iter::from_fn(|| words.next()).map(|word| entity(Some(word))).collect::<anyhow::Result<Vec<_>>>()?;
            let Response::Scene(text) = client.request(Request::Save { roots })? else { bail!("unexpected response") };
            print!("{text}");
        },
        other => bail!("unknown command `{other}`, see `help`")
    }
    Ok(())
}

/// Splits a command line into words, leaving the rest for a value.
struct Words<'a>(&'a str);

impl <'a> Words<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let rest = self.0.trim_start();
        if rest.is_empty() { return None }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        self.0 = &rest[end..];
        Some(&rest[..end])
    }

    fn rest(&self) -> &'a str { self.0.trim() }
}

fn entity(word: Option<&str>) -> anyhow::Result<EntityID> {
    let word = word.context("expected an entity id")?;
    word.parse().with_context(|| format!("`{word}` is not an entity id"))
}

/// A `<entity> <Type>` or `@<Type>` target.
fn target(words: &mut Words) -> anyhow::Result<Target> {
    let first = words.next().context("expected `<entity> <Type>` or `@<Type>`")?;
    if let Some(type_name) = first.strip_prefix('@') {
        return Ok(Target::Resource { type_name: type_name.into() })
    }
    let entity = entity(Some(first))?;
    let type_name = words.next().context("expected a component type after the entity")?.into();
    Ok(Target::Component { entity, type_name })
}

/// A path, where `.` or nothing is the whole value.
fn path(word: Option<&str>) -> String {
    match word { Some(".") | None => String::new(), Some(path) => path.into() }
}

/// A component or resource as a scene writes it, such as `Health(5)` or `Pos { x: 1.0 }`.
fn reflected(reflected: &Reflected) -> String {
    match &reflected.value {
        None => format!("{} (not reflected)", reflected.name),
        Some(Value::Unit) => reflected.name.clone(),
        Some(value @ Value::Tuple(_)) => format!("{}{value}", reflected.name),
        Some(value @ Value::Struct(_)) => format!("{} {value}", reflected.name),
        Some(value) => format!("{} = {value}", reflected.name)
    }
}
