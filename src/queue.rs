use std::cmp::Ordering;
use std::sync::{Arc, RwLock};

use rand::prelude::*;
use strum_macros::Display;

use crate::spotify::Spotify;
use crate::track::Track;

#[derive(Display, Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum RepeatSetting {
    None,
    RepeatPlaylist,
    RepeatTrack,
}

pub struct Queue {
    pub queue: Arc<RwLock<Vec<Track>>>,
    random_order: RwLock<Option<Vec<usize>>>,
    current_track: RwLock<Option<usize>>,
    repeat: RwLock<RepeatSetting>,
    spotify: Arc<Spotify>,
}

impl Queue {
    pub fn new(spotify: Arc<Spotify>) -> Queue {
        let q = Queue {
            queue: Arc::new(RwLock::new(Vec::new())),
            spotify,
            current_track: RwLock::new(None),
            repeat: RwLock::new(RepeatSetting::None),
            random_order: RwLock::new(None),
        };
        q.set_repeat(q.spotify.repeat);
        q.set_shuffle(q.spotify.shuffle);
        q
    }

    pub fn next_index(&self) -> Option<usize> {
        match *self.current_track.read().unwrap() {
            Some(mut index) => {
                let random_order = self.random_order.read().unwrap();
                if let Some(order) = random_order.as_ref() {
                    index = order.iter().position(|&i| i == index).unwrap();
                }

                let mut next_index = index + 1;
                if next_index < self.queue.read().unwrap().len() {
                    if let Some(order) = random_order.as_ref() {
                        next_index = order[next_index];
                    }

                    Some(next_index)
                } else {
                    None
                }
            }
            None => None,
        }
    }

    pub fn previous_index(&self) -> Option<usize> {
        match *self.current_track.read().unwrap() {
            Some(mut index) => {
                let random_order = self.random_order.read().unwrap();
                if let Some(order) = random_order.as_ref() {
                    index = order.iter().position(|&i| i == index).unwrap();
                }

                if index > 0 {
                    let mut next_index = index - 1;
                    if let Some(order) = random_order.as_ref() {
                        next_index = order[next_index];
                    }

                    Some(next_index)
                } else {
                    None
                }
            }
            None => None,
        }
    }

    pub fn get_current(&self) -> Option<Track> {
        match self.get_current_index() {
            Some(index) => Some(self.queue.read().unwrap()[index].clone()),
            None => None,
        }
    }

    pub fn get_current_index(&self) -> Option<usize> {
        *self.current_track.read().unwrap()
    }

    pub fn append(&self, track: &Track) {
        let mut random_order = self.random_order.write().unwrap();
        if let Some(order) = random_order.as_mut() {
            let index = order.len().saturating_sub(1);
            order.push(index);
        }

        let mut q = self.queue.write().unwrap();
        q.push(track.clone());
    }

    pub fn append_next(&self, tracks: Vec<&Track>) -> usize {
        let mut q = self.queue.write().unwrap();

        {
            let mut random_order = self.random_order.write().unwrap();
            if let Some(order) = random_order.as_mut() {
                order.extend((q.len().saturating_sub(1))..(q.len() + tracks.len()));
            }
        }

        let first = match *self.current_track.read().unwrap() {
            Some(index) => index + 1,
            None => q.len(),
        };

        let mut i = first;
        for track in tracks {
            q.insert(i, track.clone());
            i += 1;
        }

        first
    }

    pub fn remove(&self, index: usize) {
        {
            let mut q = self.queue.write().unwrap();
            q.remove(index);
        }

        // if the queue is empty stop playback
        let len = self.queue.read().unwrap().len();
        if len == 0 {
            self.stop();
            return;
        }

        // if we are deleting the currently playing track, play the track with
        // the same index again, because the next track is now at the position
        // of the one we deleted
        let current = *self.current_track.read().unwrap();
        if let Some(current_track) = current {
            match current_track.cmp(&index) {
                Ordering::Equal => {
                    // stop playback if we have the deleted the last item and it
                    // was playing
                    if current_track == len {
                        self.stop();
                    } else {
                        self.play(index, false, false);
                    }
                }
                Ordering::Greater => {
                    let mut current = self.current_track.write().unwrap();
                    current.replace(current_track - 1);
                }
                _ => (),
            }
        }

        if self.get_shuffle() {
            self.generate_random_order();
        }
    }

    pub fn clear(&self) {
        self.stop();

        let mut q = self.queue.write().unwrap();
        q.clear();

        let mut random_order = self.random_order.write().unwrap();
        if let Some(o) = random_order.as_mut() {
            o.clear()
        }
    }

    pub fn len(&self) -> usize {
        self.queue.read().unwrap().len()
    }

    pub fn shift(&self, from: usize, to: usize) {
        let mut queue = self.queue.write().unwrap();
        let item = queue.remove(from);
        queue.insert(to, item);

        // if the currently playing track is affected by the shift, update its
        // index
        let mut current = self.current_track.write().unwrap();
        if let Some(index) = *current {
            if index == from {
                current.replace(to);
            } else if index == to && from > index {
                current.replace(to + 1);
            } else if index == to && from < index {
                current.replace(to - 1);
            }
        }
    }

    pub fn play(&self, mut index: usize, reshuffle: bool, shuffle_index: bool) {
        if shuffle_index && self.get_shuffle() {
            let mut rng = rand::thread_rng();
            index = rng.gen_range(0, &self.queue.read().unwrap().len());
        }

        if let Some(track) = &self.queue.read().unwrap().get(index) {
            self.spotify.load(&track);
            let mut current = self.current_track.write().unwrap();
            current.replace(index);
            self.spotify.play();
            self.spotify.update_track();
        }

        if reshuffle && self.get_shuffle() {
            self.generate_random_order()
        }
    }

    pub fn toggleplayback(&self) {
        self.spotify.toggleplayback();
    }

    pub fn stop(&self) {
        let mut current = self.current_track.write().unwrap();
        *current = None;
        self.spotify.stop();
    }

    pub fn next(&self, manual: bool) {
        let q = self.queue.read().unwrap();
        let current = *self.current_track.read().unwrap();
        let repeat = *self.repeat.read().unwrap();

        if repeat == RepeatSetting::RepeatTrack && !manual {
            if let Some(index) = current {
                self.play(index, false, false);
            }
        } else if let Some(index) = self.next_index() {
            self.play(index, false, false);
        } else if repeat == RepeatSetting::RepeatPlaylist && q.len() > 0 {
            let random_order = self.random_order.read().unwrap();
            self.play(
                random_order.as_ref().map(|o| o[0]).unwrap_or(0),
                false,
                false,
            );
        } else {
            self.spotify.stop();
        }
    }

    pub fn previous(&self) {
        if let Some(index) = self.previous_index() {
            self.play(index, false, false);
        } else {
            self.spotify.stop();
        }
    }

    pub fn get_repeat(&self) -> RepeatSetting {
        let repeat = self.repeat.read().unwrap();
        *repeat
    }

    pub fn set_repeat(&self, new: RepeatSetting) {
        let mut repeat = self.repeat.write().unwrap();
        *repeat = new;
    }

    pub fn get_shuffle(&self) -> bool {
        let random_order = self.random_order.read().unwrap();
        random_order.is_some()
    }

    fn generate_random_order(&self) {
        let q = self.queue.read().unwrap();
        let mut order: Vec<usize> = Vec::with_capacity(q.len());
        let mut random: Vec<usize> = (0..q.len()).collect();

        if let Some(current) = *self.current_track.read().unwrap() {
            order.push(current);
            random.remove(current);
        }

        let mut rng = rand::thread_rng();
        random.shuffle(&mut rng);
        order.extend(random);

        let mut random_order = self.random_order.write().unwrap();
        *random_order = Some(order);
    }

    pub fn set_shuffle(&self, new: bool) {
        if new {
            self.generate_random_order();
        } else {
            let mut random_order = self.random_order.write().unwrap();
            *random_order = None;
        }
    }

    pub fn get_spotify(&self) -> Arc<Spotify> {
        self.spotify.clone()
    }
}
