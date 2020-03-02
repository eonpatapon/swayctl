extern crate i3ipc;
use clap::{App, AppSettings, Arg, SubCommand};
use i3ipc::reply;
use i3ipc::I3Connection;
use itertools::Itertools;

fn main() {
    let app = App::new("swayctl")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(
            SubCommand::with_name("bind")
                .about("Bind a workspace to an index. The destination workspace must have a name")
                .arg(
                    Arg::with_name("to")
                        .required(true)
                        .help("The destination index"),
                ),
        )
        .subcommand(
            SubCommand::with_name("rename")
                .about("Rename a workspace")
                .arg(Arg::with_name("name").required(true).help("The new name")),
        )
        .subcommand(
            SubCommand::with_name("show").about("Show a workspace").arg(
                Arg::with_name("name")
                    .required(true)
                    .help("A workspace name"),
            ),
        )
        .subcommand(
            SubCommand::with_name("move")
                .about("Move a container to a workspace")
                .arg(
                    Arg::with_name("name")
                        .required(true)
                        .help("The destination workspace name"),
                ),
        )
        .subcommand(SubCommand::with_name("list").about("List all workspaces"))
        .subcommand(SubCommand::with_name("swap").about("Swap visible workspaces"));

    let matches = app.get_matches();

    let mut connection = I3Connection::connect().unwrap();
    let ws = connection.get_workspaces().unwrap();

    let ret = match matches.subcommand() {
        ("bind", Some(args)) => bind(ws, args.value_of("to").unwrap().parse().unwrap()),
        ("rename", Some(args)) => rename(ws, args.value_of("name").unwrap().to_string()),
        ("show", Some(args)) => show(ws, args.value_of("name").unwrap().to_string()),
        ("move", Some(args)) => move_to(ws, args.value_of("name").unwrap().to_string()),
        ("list", Some(_args)) => list(ws),
        ("swap", Some(_args)) => swap(ws),
        _ => Err("".to_string()),
    };

    match ret {
        Ok(Some(c)) => {
            if let Err(e) = connection.run_command(&c) {
                println!("Run command error {:?}", e)
            }
        }
        Err(e) => println!("Command error: {}", e),
        Ok(None) => (),
    }
}

// An i3 compatible command. Can contain several commands separated
// with ';'
type Command = String;

// Store workspace attributes used to identify a workspace
#[derive(PartialEq, Debug)]
pub struct Workspace {
    pub num: Option<i32>,
    pub name: Option<String>,
}

impl Workspace {
    fn new(ws: &reply::Workspace) -> Workspace {
        let mut parts = ws.name.split(": ");
        match (parts.next(), parts.next()) {
            (Some(_), None) => {
                if ws.name == ws.num.to_string() {
                    Workspace {
                        num: Some(ws.num),
                        name: None,
                    }
                } else {
                    Workspace {
                        num: None,
                        name: Some(ws.name.to_string()),
                    }
                }
            }
            (Some("-1"), Some(name)) => Workspace {
                num: None,
                name: Some(name.to_string()),
            },
            (Some(_), Some(name)) => Workspace {
                num: Some(ws.num),
                name: Some(name.to_string()),
            },
            (None, _) => Workspace {
                num: None,
                name: None,
            },
        }
    }
    /// id returns an id to uniquely identify a workspace based on its attributes
    fn id(&self) -> String {
        let mut id = Vec::new();
        if let Some(num) = self.num {
            id.push(num.to_string())
        };
        if let Some(name) = &self.name {
            id.push(name.to_string())
        };
        id.join(": ")
    }
    fn move_to(&self, dest: &Workspace) -> String {
        format!("rename workspace {} to {}", self.id(), dest.id())
    }
}

fn find_or_create(ws: reply::Workspaces, name: String) -> Workspace {
    ws.workspaces
        .iter()
        .map(|w| Workspace::new(w))
        .find(|w| w.name.as_ref().map(|x| x == &name).unwrap_or(false))
        .unwrap_or(Workspace {
            num: None,
            name: Some(name),
        })
}

fn move_to(ws: reply::Workspaces, name: String) -> Result<Option<Command>, String> {
    let w = find_or_create(ws, name);
    Ok(Some(format!("move container to workspace {}", w.id())))
}

fn show(ws: reply::Workspaces, name: String) -> Result<Option<Command>, String> {
    let w = find_or_create(ws, name);
    Ok(Some(format!("workspace {}", w.id())))
}

fn list(ws: reply::Workspaces) -> Result<Option<Command>, String> {
    let names: Vec<String> = ws
        .workspaces
        .iter()
        .map(|w| Workspace::new(w))
        .filter(|w| w.name.is_some())
        .map(|w| w.name.unwrap())
        .sorted()
        .collect();

    println!("{}", names.join("\n"));
    Ok(None)
}

fn rename(ws: reply::Workspaces, name: String) -> Result<Option<Command>, String> {
    let current = ws
        .workspaces
        .iter()
        .find(|&w| w.focused)
        .map(|w| Workspace::new(w))
        .unwrap();

    let already_exist = ws
        .workspaces
        .iter()
        .map(|w| Workspace::new(w))
        .find(|w| w.name == Some(name.to_string()));

    if already_exist.is_some() {
        return Err(format!("a workspace named {} already exists", name));
    };

    let renamed = Workspace {
        num: current.num,
        name: Some(name),
    };
    let cmd = format!("rename workspace to \"{}\"", renamed.id());
    Ok(Some(cmd))
}

fn bind(ws: reply::Workspaces, to: i32) -> Result<Option<Command>, String> {
    let mut cmds = Vec::new();

    let current = ws
        .workspaces
        .iter()
        .find(|&w| w.focused)
        .map(|w| Workspace::new(w))
        .unwrap();

    // If the destination is the current position, do nothing
    if let Some(num) = current.num {
        if num == to {
            return Ok(None);
        }
    }

    let dest = ws
        .workspaces
        .iter()
        .find(|&w| w.num == to)
        .map(|w| Workspace::new(w));

    let new = Workspace {
        num: Some(to),
        name: current.name.clone(),
    };

    // If the destination workspace already exists, we first rename
    // the destination workspace with a temporary name to free its
    // index. We can then move the current workspace to the
    // destination index. Finally, we move the temporary named
    // workspace to the current index.
    if let Some(d) = dest {
        // If the destination index is bound to a not named workspace,
        // we just skip this binding. If we don't, we could loose the
        // destination workspace (no bound anymore and no name).
        if let None = d.name {
            return Err("the destination index is bound to a not named workspace".to_string());
        }

        let tmp = Workspace {
            num: None,
            name: Some("internal-tmp-swapping".to_string()),
        };
        cmds.push(d.move_to(&tmp));

        cmds.push(current.move_to(&new));

        let swap = Workspace {
            num: current.num,
            name: d.name,
        };
        cmds.push(tmp.move_to(&swap));
    }
    // Otherwise, just move the current workspace to the destination
    else {
        cmds.push(current.move_to(&new));
    }
    Ok(Some(cmds.join("; ")))
}

fn swap(ws: reply::Workspaces) -> Result<Option<Command>, String> {
    let mut cmds = Vec::new();

    let visible = ws
        .workspaces
        .iter()
        .filter(|&w| w.visible)
        .collect::<Vec<&i3ipc::reply::Workspace>>();

    if visible.len() != 2 {
        return Ok(None);
    }

    let current = visible.iter().find(|&w| w.focused).unwrap();
    let other = visible.iter().find(|&w| !w.focused).unwrap();

    cmds.push(format!("move workspace to output {}", other.output));
    cmds.push(format!("workspace {}", other.name));
    cmds.push(format!("move workspace to output {}", current.output));

    Ok(Some(cmds.join("; ")))
}

#[test]
fn new_workspaces() {
    fn dummy_ws(num: i32, name: &str) -> reply::Workspace {
        reply::Workspace {
            num: num,
            name: name.to_string(),
            visible: true,
            focused: true,
            urgent: false,
            rect: (0, 0, 0, 0),
            output: "".to_string(),
        }
    }
    assert_eq!(
        Workspace::new(&dummy_ws(1, "1")),
        Workspace {
            num: Some(1),
            name: None
        }
    );
    assert_eq!(
        Workspace::new(&dummy_ws(1, "1: mail")),
        Workspace {
            num: Some(1),
            name: Some("mail".to_string())
        }
    );
    assert_eq!(
        Workspace::new(&dummy_ws(1, "mail")),
        Workspace {
            num: None,
            name: Some("mail".to_string())
        }
    );
    assert_eq!(
        Workspace::new(&dummy_ws(-1, "-1: mail")),
        Workspace {
            num: None,
            name: Some("mail".to_string())
        }
    )
}
