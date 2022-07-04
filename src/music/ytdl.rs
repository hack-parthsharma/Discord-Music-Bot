use std::io::Read;
use std::process::{Command, Stdio};

pub fn get_title(uri: &str) -> Result<String, ()> {
    let ytdl_args = ["-e", "--no-playlist", uri];

    let youtube_dl = Command::new("youtube-dl")
        .args(&ytdl_args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let output = &mut String::new();
    let _ = youtube_dl.stdout.ok_or(())?.read_to_string(output);
    if !output.is_empty() {
        Ok(output.trim().to_string())
    } else {
        Err(())
    }
}
