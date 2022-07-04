use crate::configuration::ConfigLoader;
use crate::configuration::CONF_AUTOPLAYLIST_PATH;
use crate::music::playlist;
use serenity::client::bridge::voice::ClientVoiceManager;
use serenity::client::Context;
use serenity::framework::standard::macros::command;
use serenity::framework::standard::macros::group;
use serenity::framework::standard::{Args, CommandResult};
use serenity::framework::StandardFramework;
use serenity::model::channel::ChannelType::Voice;
use serenity::model::channel::{GuildChannel, Message};
use serenity::model::id::ChannelId;
use serenity::model::id::GuildId;
use serenity::prelude::Mutex;
use serenity::prelude::RwLock;
use serenity::prelude::ShareMap;
use serenity::prelude::TypeMapKey;
use serenity::voice;
use serenity::voice::Handler;
use serenity::voice::LockedAudio;
use serenity::Client;
use std::cmp;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

struct VoiceManagerProperties {
    playlist: playlist::Playlist,
    current_audio: Option<(String, LockedAudio)>,
}

impl VoiceManagerProperties {
    fn new(config_loader: &mut ConfigLoader) -> VoiceManagerProperties {
        VoiceManagerProperties {
            playlist: playlist::Playlist::new(&CONF_AUTOPLAYLIST_PATH.get_value(config_loader)),
            current_audio: None,
        }
    }
}

struct VoiceManager {
    voice_manager: Arc<Mutex<ClientVoiceManager>>,
    audio_monitor_active: Arc<AtomicBool>,
    audio_monitor: Option<JoinHandle<()>>,
    properties: Arc<Mutex<HashMap<GuildId, VoiceManagerProperties>>>,
}

impl TypeMapKey for VoiceManager {
    type Value = Arc<Mutex<VoiceManager>>;
}

group!({
    name: "voice_client",
    options: {},
    commands: [summon, leave, play, queue, skip]
});

pub fn register_module(client: &mut Client, standard_framework: &mut StandardFramework) {
    let audio_monitor_active = Arc::new(AtomicBool::new(true));
    let audio_monitor_active_clone = audio_monitor_active.clone();
    let properties_map = Arc::new(Mutex::new(HashMap::new()));
    let properties_map_clone = properties_map.clone();
    let cache_and_http = client.cache_and_http.clone();

    let voice_manager = Arc::new(Mutex::new(VoiceManager {
        voice_manager: client.voice_manager.clone(),
        audio_monitor_active,
        audio_monitor: None,
        properties: properties_map,
    }));
    let voice_manager_clone = voice_manager.clone();

    voice_manager.lock().audio_monitor = Some(thread::spawn(move || {
        let active = audio_monitor_active_clone.clone();
        while active.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_secs(1));
            let mut properties = properties_map_clone.lock();
            for (guild_id, property) in &mut *properties {
                let is_playing = match property.current_audio.as_ref() {
                    Some((_, audio)) => !audio.lock().finished,
                    None => false,
                };

                if !is_playing && active.load(Ordering::SeqCst) {
                    let client_voice_manager = {
                        let voice_manager = voice_manager_clone.lock();
                        voice_manager.voice_manager.clone()
                    };
                    let mut client_voice_manager_locked = client_voice_manager.lock();
                    if let Some(handler) = client_voice_manager_locked.get_mut(guild_id) {
                        match play_music(handler, &mut property.playlist) {
                            Ok((locked_audio, title, channel_id)) => {
                                property.current_audio = Some((title.clone(), locked_audio));
                                if let Some(id) = channel_id {
                                    let _ = id.say(
                                        cache_and_http.http.clone(),
                                        format!("```Playing \"{}\"```", title),
                                    );
                                }
                            }
                            Err((error, channel_id)) => {
                                if let Some(id) = channel_id {
                                    let _ = id.say(cache_and_http.http.clone(), error);
                                }
                            }
                        }
                    }
                }
            }
        }
    }));

    let mut data = client.data.write();

    if data.get::<VoiceManager>().is_none() {
        data.insert::<VoiceManager>(voice_manager.clone());
    } else {
        panic!("VoiceManager already inserted");
    }
    standard_framework.group_add(&VOICE_CLIENT_GROUP);
}

pub fn unregister_module(data: Arc<RwLock<ShareMap>>) {
    let thread = {
        let mut share_map = data.write();
        let mut voice_manager = share_map
            .get_mut::<VoiceManager>()
            .expect("Expected VoiceManager in ShareMap.")
            .lock();

        voice_manager
            .audio_monitor_active
            .store(false, Ordering::SeqCst);
        voice_manager.audio_monitor.take().unwrap()
    };
    thread.join().unwrap();
}

#[command]
pub fn summon(ctx: &mut Context, msg: &Message, args: Args) -> CommandResult {
    let guild = match msg.guild(&ctx.cache) {
        Some(guild) => guild,
        None => return Ok(()),
    };

    let guild_id = guild.read().id;

    let channel_id: ChannelId = match args.remains() {
        Some(channel_name) => {
            let channels = &guild.read().channels;
            let matching_channels: Vec<&Arc<RwLock<GuildChannel>>> = channels
                .values()
                .filter(|channel| {
                    channel.read().kind == Voice
                        && channel.read().name.to_lowercase() == channel_name.to_lowercase()
                })
                .collect();

            match matching_channels.first() {
                Some(channel) => channel.read().id,
                None => {
                    msg.channel_id
                        .say(&ctx.http, format!("```{} does not exist```", channel_name))?;
                    return Ok(());
                }
            }
        }
        None => match guild
            .read()
            .voice_states
            .get(&msg.author.id)
            .and_then(|voice_state| voice_state.channel_id)
        {
            Some(channel_id) => channel_id,
            None => {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, "```You are not in a voice channel```");
                return Ok(());
            }
        },
    };

    let voice_manager = {
        let mut share_map = ctx.data.write();
        share_map
            .get_mut::<VoiceManager>()
            .expect("Expected Scheduler.")
            .clone()
    };

    let config_loader = {
        let mut share_map = ctx.data.write();
        share_map
            .get_mut::<ConfigLoader>()
            .expect("Expected Dispatcher.")
            .clone()
    };

    let voice_manager_locked = voice_manager.lock();
    let client_voice_manager = voice_manager_locked.voice_manager.clone();

    match client_voice_manager.lock().join(guild_id, channel_id) {
        Some(_) => {
            let _ = msg.channel_id.say(
                &ctx.http,
                &format!("```Joined {}```", channel_id.name(&ctx.cache).unwrap()),
            );
            let mut properties = voice_manager_locked.properties.lock();
            properties
                .entry(guild_id)
                .or_insert_with(|| VoiceManagerProperties::new(&mut config_loader.lock()));
        }
        None => {
            let _ = msg
                .channel_id
                .say(&ctx.http, "```Error joining the channel```");
            return Ok(());
        }
    }

    Ok(())
}

#[command]
pub fn leave(ctx: &mut Context, msg: &Message) -> CommandResult {
    let guild_id = match ctx.cache.read().guild_channel(msg.channel_id) {
        Some(channel) => channel.read().guild_id,
        None => return Ok(()),
    };

    let share_map = ctx.data.read();
    let manager_lock = share_map
        .get::<VoiceManager>()
        .expect("Expected VoiceManager in ShareMap.")
        .lock();
    let mut manager = manager_lock.voice_manager.lock();

    match manager.get(guild_id) {
        Some(handler) => {
            if let Some(channel_id) = handler.channel_id {
                let _ = msg.channel_id.say(
                    &ctx.http,
                    format!("```Left {}```", channel_id.name(&ctx.cache).unwrap()),
                );
            } else {
                let _ = msg.channel_id.say(&ctx.http, "```Left voice channel```");
            }
            manager.leave(guild_id);
        }
        None => {
            let _ = msg.channel_id.say(&ctx, "```Not in a voice channel```");
        }
    }

    Ok(())
}

#[command]
pub fn play(ctx: &mut Context, msg: &Message, mut args: Args) -> CommandResult {
    let url = match args.single::<String>() {
        Ok(url) => url,
        Err(_) => {
            msg.channel_id
                .say(&ctx.http, "```Must provide a URL to a video or audio```")?;
            return Ok(());
        }
    };

    if !url.starts_with("http") {
        msg.channel_id
            .say(&ctx.http, "```Must provide a valid URL```")?;
        return Ok(());
    }

    let guild = match msg.guild(&ctx.cache) {
        Some(guild) => guild,
        None => return Ok(()),
    };

    let guild_id = guild.read().id;

    let user_channel_id = match guild
        .read()
        .voice_states
        .get(&msg.author.id)
        .and_then(|voice_state| voice_state.channel_id)
    {
        Some(channel_id) => channel_id,
        None => {
            msg.channel_id
                .say(&ctx.http, "```You must be in a voice channel```")?;
            return Ok(());
        }
    };

    let voice_manager = {
        let share_map = ctx.data.read();
        share_map
            .get::<VoiceManager>()
            .expect("Expected VoiceManager in ShareMap.")
            .clone()
    };

    let config_loader = {
        let share_map = ctx.data.read();
        share_map
            .get::<ConfigLoader>()
            .expect("Expected ConfigLoader in ShareMap.")
            .clone()
    };

    let voice_manager_locked = voice_manager.lock();
    let mut manager = voice_manager_locked.voice_manager.lock();
    let mut properties = voice_manager_locked.properties.lock();

    match manager.get_mut(guild_id) {
        Some(handler) => {
            if handler.channel_id.unwrap() != user_channel_id {
                handler.switch_to(user_channel_id);
            }
        }
        None => {
            if manager.join(guild_id, user_channel_id).is_none() {
                let _ = msg.channel_id.say(&ctx.http, "```Can't join channel```");
            }
        }
    };

    let property = properties
        .entry(guild_id)
        .or_insert_with(|| VoiceManagerProperties::new(&mut config_loader.lock()));
    match property
        .playlist
        .push(url.to_string(), Some(msg.channel_id))
    {
        Ok(title) => {
            let _ = msg
                .channel_id
                .say(&ctx.http, format!("```Added \"{}\" to queue```", title));
        }
        Err(_) => {
            let _ = msg
                .channel_id
                .say(&ctx.http, format!("```Couldn't play {}```", url));
        }
    }

    Ok(())
}

#[command]
pub fn queue(ctx: &mut Context, msg: &Message) -> CommandResult {
    let voice_manager = {
        let mut share_map = ctx.data.write();
        share_map
            .get_mut::<VoiceManager>()
            .expect("Expected VoiceManager in ShareMap.")
            .clone()
    };

    let config_loader = {
        let mut share_map = ctx.data.write();
        share_map
            .get_mut::<ConfigLoader>()
            .expect("Expected ConfigLoader in ShareMap.")
            .clone()
    };

    let voice_manager_properties = voice_manager.lock().properties.clone();
    let mut voice_manager_properties_locked = voice_manager_properties.lock();
    let property = voice_manager_properties_locked
        .entry(msg.guild_id.unwrap())
        .or_insert_with(|| VoiceManagerProperties::new(&mut config_loader.lock()));

    let queue = property.playlist.get_queue();
    let mut output = "```".to_string();
    if let Some((title, _)) = &property.current_audio {
        output.push_str(&format!("Playing right now \"{}\"\n\n", title));
    }
    for title in queue.iter().take(cmp::min(5, queue.len())) {
        output.push_str(title);
        output.push_str("\n\n");
    }
    output.push_str("```");
    let _ = msg.channel_id.say(&ctx.http, output);

    Ok(())
}

#[command]
pub fn skip(ctx: &mut Context, msg: &Message) -> CommandResult {
    let voice_manager = {
        let mut share_map = ctx.data.write();
        share_map
            .get_mut::<VoiceManager>()
            .expect("Expected VoiceManager in ShareMap.")
            .clone()
    };

    let config_loader = {
        let mut share_map = ctx.data.write();
        share_map
            .get_mut::<ConfigLoader>()
            .expect("Expected ConfigLoader in ShareMap.")
            .clone()
    };

    let guild_id = msg.guild_id.unwrap();

    let voice_manager_locked = voice_manager.lock();
    let mut manager = voice_manager_locked.voice_manager.lock();
    let mut voice_manager_properties_locked = voice_manager_locked.properties.lock();
    let property = voice_manager_properties_locked
        .entry(guild_id)
        .or_insert_with(|| VoiceManagerProperties::new(&mut config_loader.lock()));

    property.current_audio = None;

    if let Some(handler) = manager.get_mut(guild_id) {
        handler.stop();
    };

    Ok(())
}

type MusicResult = (LockedAudio, String, Option<ChannelId>);

fn play_music(
    handler: &mut Handler,
    playlist: &mut playlist::Playlist,
) -> Result<MusicResult, (String, Option<ChannelId>)> {
    handler.deafen(true);
    if let Some((title, url, channel_id)) = playlist.poll() {
        match voice::ytdl(&url) {
            Ok(source) => {
                println!("{}: Start playing {} {}", handler.guild_id, title, url);
                let audio = handler.play_only(source);
                return Ok((audio, title, channel_id));
            }
            Err(_) => return Err((format!("Couldn't play {}", title), channel_id)),
        }
    }
    Err(("".to_string(), None))
}
