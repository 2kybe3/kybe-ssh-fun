use std::env;

use rustyline::{DefaultEditor, error::ReadlineError};

const PGP_KEY: &str = include_str!("../static/pgp.txt");

fn main() -> anyhow::Result<()> {
    let mut rl = DefaultEditor::new()?;
    let args: Vec<String> = env::args().collect();

    if let Some(args) = args.get(2) {
        let args: Vec<String> = args.split(" ").map(|s| s.to_owned()).collect();

        if let Some(arg) = args.first() {
            match arg.as_str() {
                "gpg" | "pgp" =>  {
                    println!("{}", PGP_KEY);
                    return Ok(());
                }
                _ => {
                    println!("invalid arg supplied");
                    return Ok(());
                }
            }
        }
    }

    println!("Hello Stranger!");

    loop {
        let input = rl.readline("kybe>> ");
        match input {
            Ok(line) => match line.trim() {
                "exit" | "q" | "quit" => {
                    println!("bye");
                    break;
                }
                "pgp" => println!("{}", PGP_KEY),
                "help" => println!("Avaible commands: help, pgp, exit"),
                "" => continue,
                _ => println!("Unknown command: {}", line),
            },
            Err(ReadlineError::Interrupted) => {
                println!("bye");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    Ok(())
}
