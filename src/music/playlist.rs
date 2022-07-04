extern crate rand;
use crate::music::ytdl;
use rand::Rng;
use serenity::model::id::ChannelId;
use serenity::prelude::RwLock;
use std::cmp;
use std::collections::HashSet;
use std::fs::File;
use std::io::prelude::BufRead;
use std::io::BufReader;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;

type Title = (String, String, Option<ChannelId>);

pub(crate) struct Playlist {
    autoplaylist: Arc<RwLock<HashSet<String>>>,
    remaining_autoplaylist: Arc<RwLock<Vec<String>>>,
    regular_queue: Arc<RwLock<Vec<Title>>>,
    autoplaylist_queue: Arc<RwLock<Vec<(String, String)>>>,
}

impl Playlist {
    pub(crate) fn new(path: &str) -> Self {
        let autoplaylist = Arc::new(RwLock::new(HashSet::<String>::new()));

        if !path.is_empty() {
            if let Ok(file) = File::open(path) {
                let reader = BufReader::new(file);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        if line.starts_with("http") {
                            autoplaylist.write().insert(line);
                        }
                    }
                }
            }
        }

        let remaining_autoplaylist = Arc::new(RwLock::new(Vec::<String>::new()));
        Playlist::fill_remaining_autoplaylist(
            &*autoplaylist.read(),
            &mut *remaining_autoplaylist.write(),
        );

        let autoplaylist_queue = Arc::new(RwLock::new(Vec::<(String, String)>::new()));
        let mut remaining_count = cmp::min(5, autoplaylist.read().len());
        let mut rng = rand::thread_rng();
        while remaining_count > 0 {
            let mut threads = Vec::<(usize, String, JoinHandle<Option<String>>)>::new();
            let autoplaylist_length = remaining_autoplaylist.read().len();
            for _ in 0..remaining_count {
                let random_index = rng.gen_range(usize::min_value(), autoplaylist_length);
                let url = remaining_autoplaylist.read()[random_index].clone();
                threads.push((
                    random_index,
                    url.clone(),
                    thread::spawn(move || match ytdl::get_title(&url) {
                        Ok(title) => Some(title),
                        Err(_) => None,
                    }),
                ));
            }
            for (index, url, thread) in threads {
                remaining_autoplaylist.write().remove(index);
                match thread.join().unwrap() {
                    Some(title) => {
                        autoplaylist_queue.write().push((title, url));
                        remaining_count -= 1;
                    }
                    None => {
                        autoplaylist.write().remove(&url);
                    }
                }
            }
        }

        Playlist {
            autoplaylist,
            remaining_autoplaylist: remaining_autoplaylist.clone(),
            regular_queue: Arc::new(RwLock::new(Vec::new())),
            autoplaylist_queue,
        }
    }

    fn fill_remaining_autoplaylist(
        autoplaylist: &HashSet<String>,
        remaining_autoplaylist: &mut Vec<String>,
    ) {
        if remaining_autoplaylist.is_empty() {
            for url in autoplaylist {
                remaining_autoplaylist.push(url.clone());
            }
        }
    }

    pub(crate) fn get_queue(&self) -> Vec<String> {
        let regular_queue = self.regular_queue.read();
        let autoplaylist_queue = self.autoplaylist_queue.read();
        let mut queue = Vec::<String>::new();
        for (title, _, _) in &*regular_queue {
            queue.push(title.clone());
        }
        for (title, _) in &*autoplaylist_queue {
            queue.push(title.clone());
        }
        queue
    }

    pub(crate) fn poll(&mut self) -> Option<(String, String, Option<ChannelId>)> {
        {
            let mut regular_queue = self.regular_queue.write();
            if !regular_queue.is_empty() {
                return Some(regular_queue.remove(0));
            }
        }

        if !self.autoplaylist.read().is_empty() {
            let next = {
                let mut autoplaylist_queue = self.autoplaylist_queue.write();
                let removed = autoplaylist_queue.remove(0);
                Some((removed.0, removed.1, None))
            };

            let autoplaylist_clone = self.autoplaylist.clone();
            let remaining_autoplaylist_clone = self.remaining_autoplaylist.clone();
            let autoplaylist_queue_clone = self.autoplaylist_queue.clone();
            thread::spawn(move || loop {
                let mut rng = rand::thread_rng();
                let mut remaining_autoplaylist = remaining_autoplaylist_clone.write();
                let mut autoplaylist_queue = autoplaylist_queue_clone.write();
                if remaining_autoplaylist.is_empty() {
                    Playlist::fill_remaining_autoplaylist(
                        &*autoplaylist_clone.read(),
                        &mut remaining_autoplaylist,
                    );
                }
                let random_index = rng.gen_range(usize::min_value(), remaining_autoplaylist.len());
                let url = &remaining_autoplaylist.remove(random_index);
                match ytdl::get_title(url) {
                    Ok(title) => {
                        autoplaylist_queue.push((title, url.clone()));
                        return;
                    }
                    Err(_) => {
                        autoplaylist_clone.write().remove(url);
                    }
                }
            });
            return next;
        }

        None
    }

    pub(crate) fn push(
        &mut self,
        url: String,
        channel_id: Option<ChannelId>,
    ) -> Result<String, ()> {
        let mut regular_queue = self.regular_queue.write();
        match ytdl::get_title(&url) {
            Ok(title) => {
                regular_queue.push((title.clone(), url, channel_id));
                Ok(title)
            }
            Err(_) => Err(()),
        }
    }
}
