extern crate ctrlc;
use crate::configuration::ConfigLoader;
use crate::configuration::{CONF_PREFIX, CONF_TOKEN};
use serenity::framework::StandardFramework;
use serenity::prelude::EventHandler;
use serenity::prelude::Mutex;
use serenity::Client;
use std::env;
use std::process::exit;
use std::sync::Arc;

mod configuration;
mod music;

use music::voice_client;

struct Handler;

impl EventHandler for Handler {}

fn main() {
    let mut config_loader = {
        let args: Vec<String> = env::args().collect();
        if args.len() > 2 {
            println!("Usage: ./{} [config]", args[0]);
            exit(1);
        }

        match args.len() {
            2 => configuration::ConfigLoader::new(&args[1]),
            _ => configuration::ConfigLoader::default(),
        }
    };

    let token = CONF_TOKEN.get_value(&mut config_loader);
    if token.is_empty() {
        println!("Token is empty!");
        exit(1);
    }

    let mut client = Client::new(token, Handler).expect("Couldn't create client");
    let shard_manager = client.shard_manager.clone();
    let data = client.data.clone();
    let mut standard_framework = StandardFramework::new()
        .configure(|c| c.prefix(&CONF_PREFIX.get_value(&mut config_loader)));

    data.write()
        .insert::<ConfigLoader>(Arc::new(Mutex::new(config_loader)));

    println!("Initializing...");
    println!("Voice module...");
    voice_client::register_module(&mut client, &mut standard_framework);
    println!("Voice module done");

    client.with_framework(standard_framework);

    ctrlc::set_handler(move || {
        println!("\nShutting down...");
        voice_client::unregister_module(data.clone());
        shard_manager.lock().shutdown_all();
    })
    .expect("Error setting Ctrl-C handler");

    let _ = client
        .start()
        .map_err(|why| println!("Client ended: {:?}", why));
}
