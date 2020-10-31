#![feature(async_closure)]

use anyhow::Result as AnyResult;
use serenity::{
    framework::{standard::macros::group, StandardFramework},
    http::client::Http,
    prelude::*,
};
use std::{
    fs::{self, File},
    io::{Error as IoError, ErrorKind as IoErrorKind, Write},
    sync::Arc,
    time::Duration,
};
use structopt::StructOpt;
use tokio::{
    fs::File as AsyncFile,
    io::AsyncWriteExt,
    sync::mpsc::{unbounded_channel, UnboundedReceiver},
    time::interval,
};

mod anagram;
mod args;
mod cache_keys;
mod config;
mod current_gulags;
mod gulag;
mod handler;
mod misc;
mod tasks;

use anagram::*;
use cache_keys::*;
use config::Config;
use current_gulags::*;
use gulag::*;
use handler::{after, Handler};
use tasks::TaskType;

pub const FILES_DIR: &str = "files/";

/*
mod gulag;
use gulag::*;
mod help;
use help::Help;
mod gulag_handling;
use gulag_handling::*;
mod remove_gulag_info;
use remove_gulag_info::RemoveGulagInfo;
use anagram::Anagram;
mod reginald;
use reginald::reginald_visits;
mod kirb_day;
use kirb_day::kirb_day_task;
mod source;
use source::Source;

pub const COUNTER_FILE: &str = "./activity_counter";
pub const GULAG_DIR: &str = "./gulags";
pub const EMBED_ICON_URL: &str = "https://cdn.discordapp.com/avatars/555257721587499038/\
    d1e248dc6720d3484c97bab2bf03e75f.png";
pub const GATHERING_PERIOD: u64 = 604800; // one week in seconds
pub const CRAK_UID: UserId = UserId(221345168463364098);
pub const BOT_UID: UserId = UserId(555257721587499038);
pub const SHIT_CHANNEL: ChannelId = ChannelId(549383666246090773);
pub const ANNOUNCEMENTS_CHANNEL: ChannelId = ChannelId(549385011107987477);
pub const AXOLOTL_ARMADA_GID: GuildId = GuildId(549382175703957504);

pub const WEEK_AS_SECS: u64 = 604800;
pub const DAY_AS_SECS: u64 = 86400;
pub const HOUR_AS_SECS: u64 = 3600;
pub const MIN_AS_SECS: u64 = 60;*/

#[group]
#[commands(anagram)]
struct GeneralCommands;

#[group]
#[commands(current_gulags, gulag)]
struct AdminCommands;

#[tokio::main]
async fn main() -> AnyResult<()> {
    let args::Args { config_file_path } = args::Args::from_args();
    let config_contents = match fs::read_to_string(&config_file_path) {
        Ok(contents) => Ok(contents),
        Err(error) => {
            if error.kind() == IoErrorKind::NotFound {
                println!(
                    "Config file not found. Attempting to create new default config file at '{}'",
                    config_file_path
                );
                let mut new_config_file = File::create(&config_file_path)?;
                let default_contents = serde_json::to_string_pretty(&Config::default()).unwrap();
                new_config_file.write_all(default_contents.as_bytes())?;
                println!("Created new config file and wrote defaults.");
            }
            Err(error)
        }
    }?;
    println!("Read config file contents.");
    let mut config = serde_json::from_str::<Config>(&config_contents)?;
    println!("Parsed config from config file contents.");
    let tasks = match fs::read_to_string(&config.tasks_file) {
        Ok(contents) if contents.len() == 0 => Ok(Vec::new()),
        Ok(contents) => Ok(serde_json::from_str::<Vec<TaskType>>(&contents)?),
        Err(error) => match error.kind() {
            IoErrorKind::NotFound => {
                println!(
                    "Tasks file not found. Attempting to create new tasks file at '{}'",
                    config.tasks_file
                );
                let _ = File::create(&config.tasks_file)?;
                println!("Created new blank tasks file.");
                Ok(Vec::new())
            }
            _ => Err(error),
        },
    }?;
    println!("Collected tasks.");
    let framework = StandardFramework::new()
        .configure(|c| c.prefix("=>"))
        .after(after)
        .group(&GENERALCOMMANDS_GROUP)
        .group(&ADMINCOMMANDS_GROUP);
    println!("Created framework.");
    let mut client = Client::builder(&config.bot_id)
        .framework(framework)
        .event_handler(Handler)
        .await?;
    println!("Created client.");
    // Get bot ID
    let bot_id = client
        .cache_and_http
        .http
        .get_current_application_info()
        .await?
        .id;
    // Cache bot ID
    client.data.write().await.insert::<BotIdKey>(bot_id);
    println!("Fetched and cached bot ID.");
    // Get all the roles in the guild to find the gulag role.
    let guild_roles = client
        .cache_and_http
        .http
        .get_guild_roles(config.guild_id.into())
        .await?;
    println!("Fetched guild roles.");
    // Try to find the gulag role.
    let gulag_role = guild_roles
        .iter()
        .find(|&role| role.name == config.prisoner_role_name || role.id == config.prisoner_role_id)
        .ok_or({
            let msg = format!(
                "Failed to get gulag role by name ('{}') or ID ('{}').",
                config.prisoner_role_name, config.prisoner_role_id
            );
            IoError::new(IoErrorKind::InvalidData, msg.as_str())
        })?
        .clone();
    println!("Found gulag role in guild roles.");
    println!("Checking whether it is necessary to update the prisoner role name or ID");
    // Update role name and/or ID in config if necessary, and write out to file.
    if gulag_role.id != config.prisoner_role_id || gulag_role.id != config.prisoner_role_id {
        if gulag_role.id != config.prisoner_role_id {
            println!("    IDs do not match. Updating ID.");
            config.prisoner_role_id = gulag_role.id;
        } else if gulag_role.name != config.prisoner_role_name {
            println!("    Names do not match. Updating name.");
            config.prisoner_role_name.clear();
            config.prisoner_role_name.push_str(&gulag_role.name);
        }
        println!("    Re-creating config file.");
        let mut file = File::create(&config_file_path)?;
        println!("    Serializing updated config.");
        let config_string = serde_json::to_string_pretty(&config)?;
        println!("    Writing updated config to file.");
        file.write_all(config_string.as_bytes())?;
        println!("    Updated saved config.");
    } else {
        println!("    Saved config is up to date.");
    }
    // Cache gulag role.
    client.data.write().await.insert::<GulagRoleKey>(gulag_role);
    println!("Cached gulag role.");
    // Find all the roles allowed permission to use all commands and cache them as well.
    let elevated_roles = guild_roles
        .iter()
        .filter(|&role1| {
            config
                .elevated_roles
                .iter()
                .any(|role2| &role1.name == &role2.0 || &role1.id == &role2.1)
        })
        .map(|role| role.clone())
        .collect::<Vec<_>>();
    println!("Found elevated roles.");
    println!("Checking whether it is necessary to update elevated role names or IDs");
    // Update role name and/or ID for each role in config if necessary, and write out to file.
    if elevated_roles
        .iter()
        .map(|role| {
            println!(
                "    Checking config values for role '{}' (ID {})",
                role.name, role.id
            );
            let config = config
                .elevated_roles
                .iter_mut()
                .find(|config_role| &role.id == &config_role.1 || &role.name == &config_role.0)
                .unwrap();
            if &role.id != &config.1 {
                println!("        IDs do not match. Updating ID.");
                config.1 = role.id;
                true
            } else if &role.name != &config.0 {
                println!("        Names do not match. Updating name.");
                config.0.clear();
                config.0.push_str(&role.name.as_str());
                true
            } else {
                println!("        Name and ID match.");
                false
            }
        })
        .fold(false, |acc, new| acc || new)
    {
        println!("    Re-creating config file.");
        let mut file = File::create(&config_file_path)?;
        println!("    Serializing updated config.");
        let config_string = serde_json::to_string_pretty(&config)?;
        println!("    Writing updated config to file.");
        file.write_all(config_string.as_bytes())?;
        println!("    Updated saved config.");
    } else {
        println!("    Saved config is up to date.");
    }
    // Cache elevated roles.
    client
        .data
        .write()
        .await
        .insert::<ElevatedRolesKey>(elevated_roles);
    // Cache the config.
    client.data.write().await.insert::<ConfigKey>(config);
    // Cache the tasks - they may need to be updated depending on role changes and such.
    client.data.write().await.insert::<TasksKey>(tasks);
    println!("Cached tasks.");
    // Create a channel for the bot thread to be able to send new tasks to the main thread.
    let (send, recv) = unbounded_channel();
    client.data.write().await.insert::<TaskSenderKey>(send);
    // Spawn a ctrl+c handler here and have it send the proper instructions n' stuff.
    // todo
    // Start the task handling loop in a separate thread.
    println!("Starting task handling loop.");
    let data_clone = client.data.clone();
    let http_clone = client.cache_and_http.http.clone();
    tokio::spawn(start_task_handler(data_clone, http_clone, recv));
    // Start the client.
    println!("Starting client.");
    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
    Ok(())
}

async fn start_task_handler(
    data: Arc<RwLock<TypeMap>>,
    http: Arc<Http>,
    mut recv: UnboundedReceiver<TaskType>,
) -> AnyResult<()> {
    let mut interval = interval(Duration::from_millis(500));
    loop {
        // Get write lock on context data.
        let context_data = data.read().await;
        let mut tasks = context_data.get::<TasksKey>().unwrap().clone();
        drop(context_data);
        let mut made_changes = false;
        // Check for new tasks.
        while let Ok(task) = recv.try_recv() {
            println!("TL | Received new task - pushing to task list.");
            tasks.push(task);
            made_changes = true;
        }
        // Check whether any current tasks need to be executed.
        for i in (0..tasks.len()).rev() {
            if tasks[i].time_to_act() {
                tasks[i].act(&data, &http).await?;
                if tasks[i].is_gulag() {
                    println!("TL | Gulag period has elapsed - removing from task list.");
                    tasks.remove(i);
                    made_changes = true;
                }
            }
        }
        if made_changes {
            println!("TL | Changes to task list were made.");
            println!("TL | Assigning context task list to changed list.");
            let mut context_data = data.write().await;
            *context_data.get_mut::<TasksKey>().unwrap() = tasks;
            drop(context_data);
            println!("TL | Serializing task list and writing to task file.");
            let context_data = data.read().await;
            let tasks = context_data.get::<TasksKey>().unwrap();
            let tasks_path = context_data.get::<ConfigKey>().unwrap().tasks_file.as_str();
            let mut tasks_file = AsyncFile::create(tasks_path).await?;
            let new_contents = serde_json::to_string_pretty(tasks).unwrap();
            tasks_file.write_all(new_contents.as_bytes()).await?;
        }
        interval.tick().await;
    }
}
