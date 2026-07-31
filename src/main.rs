mod meeting;

use std::{env, process};

fn main() {
    let mut arguments = env::args().skip(1);

    match arguments.next().as_deref() {
        Some("create-meeting") => {
            let title = arguments.next().unwrap_or_else(|| "untitled".to_string());

            if arguments.next().is_some() {
                print_usage();
                process::exit(2);
            }

            match meeting::create(&title) {
                Ok(folder) => println!("Created meeting folder: {}", folder.display()),
                Err(error) => {
                    eprintln!("Could not create meeting folder: {error}");
                    process::exit(1);
                }
            }
        }
        _ => print_usage(),
    }
}

fn print_usage() {
    println!("Usage: rusteze create-meeting [title]");
    println!("Example: rusteze create-meeting \"Rust workshop\"");
}
