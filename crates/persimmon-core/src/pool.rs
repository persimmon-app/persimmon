// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The bounded pool: how many tasks may run at once. Image, PDF and
//! archive tasks share up to `total` slots; video and audio take at most
//! `media` of them, since FFmpeg already uses every core. Work someone
//! started goes before background work waiting for the same lane.

use std::num::NonZeroUsize;
use std::str::FromStr;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use crate::exec::CancelToken;
use crate::registry::Backend;

/// How many tasks to run at once, as `--jobs` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jobs {
    /// One per performance core.
    Auto,
    Fixed(NonZeroUsize),
}

impl FromStr for Jobs {
    type Err = String;

    fn from_str(s: &str) -> Result<Jobs, String> {
        if s == "auto" {
            return Ok(Jobs::Auto);
        }
        s.parse::<NonZeroUsize>()
            .map(Jobs::Fixed)
            .map_err(|_| "takes a number of at least 1, or auto".to_string())
    }
}

/// The pool's bounds: at most `total` tasks, `media` of them video or audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub total: usize,
    pub media: usize,
}

impl Limits {
    /// One task at a time: today's behaviour.
    pub fn serial() -> Limits {
        Limits { total: 1, media: 1 }
    }

    pub fn of(jobs: Jobs) -> Limits {
        let total = match jobs {
            Jobs::Auto => performance_cores(),
            Jobs::Fixed(n) => n.get(),
        };
        Limits { total, media: 1 }
    }

    /// One task per performance core.
    pub fn auto() -> Limits {
        Limits::of(Jobs::Auto)
    }
}

/// The number of performance cores: on macOS what `sysctl` reports for
/// the first performance level, elsewhere (or if that fails) the number of
/// threads the system offers. Never below 1.
pub fn performance_cores() -> usize {
    static CORES: OnceLock<usize> = OnceLock::new();
    *CORES.get_or_init(|| {
        let fallback = || std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
        if cfg!(target_os = "macos") {
            std::process::Command::new("sysctl")
                .args(["-n", "hw.perflevel0.physicalcpu"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<usize>().ok())
                .filter(|&n| n >= 1)
                .unwrap_or_else(fallback)
        } else {
            fallback()
        }
    })
}

/// Which slots a task competes for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Image, PDF and archive work.
    Cpu,
    /// Video and audio: FFmpeg, one at a time.
    Media,
}

impl Lane {
    pub fn of(backend: Backend) -> Lane {
        match backend {
            Backend::Ffmpeg => Lane::Media,
            Backend::Image | Backend::Pdf | Backend::Archive => Lane::Cpu,
        }
    }
}

/// Who is waiting: someone who started the work, or a background source
/// such as a watched folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    User,
    Background,
}

#[derive(Debug, Default)]
struct State {
    running: usize,
    media: usize,
    /// `User` waiters per lane: [cpu, media].
    users_waiting: [usize; 2],
}

#[derive(Debug)]
struct Shared {
    limits: Limits,
    state: Mutex<State>,
    freed: Condvar,
}

/// The slots. Clones share them.
#[derive(Debug, Clone)]
pub struct Pool(Arc<Shared>);

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn slot(lane: Lane) -> usize {
    match lane {
        Lane::Cpu => 0,
        Lane::Media => 1,
    }
}

impl Pool {
    /// A pool with these limits. More than one slot sets libvips's own
    /// threading to one, so the CPU isn't oversubscribed.
    pub fn new(limits: Limits) -> Pool {
        let limits = Limits {
            total: limits.total.max(1),
            media: limits.media.clamp(1, limits.total.max(1)),
        };
        if limits.total > 1 {
            crate::backend::set_vips_threads(1);
        }
        Pool(Arc::new(Shared {
            limits,
            state: Mutex::new(State::default()),
            freed: Condvar::new(),
        }))
    }

    pub fn limits(&self) -> Limits {
        self.0.limits
    }

    /// How many permits are out.
    pub fn running(&self) -> usize {
        lock(&self.0.state).running
    }

    fn fits(&self, state: &State, lane: Lane) -> bool {
        state.running < self.0.limits.total
            && (lane == Lane::Cpu || state.media < self.0.limits.media)
    }

    fn take(&self, state: &mut State, lane: Lane) -> Permit {
        state.running += 1;
        if lane == Lane::Media {
            state.media += 1;
        }
        Permit {
            pool: self.clone(),
            lane,
        }
    }

    /// A slot now, if one is free for `lane`. Counts as work someone started.
    pub fn try_acquire(&self, lane: Lane) -> Option<Permit> {
        let mut state = lock(&self.0.state);
        self.fits(&state, lane).then(|| self.take(&mut state, lane))
    }

    /// Wait for a slot in `lane`. A `Background` waiter never takes a slot
    /// while a `User` waiter (or a [`Wanting`]) wants a lane that has room.
    /// `None` once `cancel` fires.
    pub fn acquire(&self, lane: Lane, priority: Priority, cancel: &CancelToken) -> Option<Permit> {
        let mut state = lock(&self.0.state);
        if priority == Priority::User {
            state.users_waiting[slot(lane)] += 1;
        }
        let permit = loop {
            if cancel.is_cancelled() {
                break None;
            }
            // Background work defers to any user whose lane has room now.
            let ahead = priority == Priority::Background
                && [Lane::Cpu, Lane::Media]
                    .into_iter()
                    .any(|l| state.users_waiting[slot(l)] > 0 && self.fits(&state, l));
            if !ahead && self.fits(&state, lane) {
                break Some(self.take(&mut state, lane));
            }
            state = self
                .0
                .freed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        };
        if priority == Priority::User {
            state.users_waiting[slot(lane)] -= 1;
            // A background waiter may now go.
            self.0.freed.notify_all();
        }
        permit
    }
}

/// A user's standing claim on a lane: while it lives, background work
/// waiting for a slot defers to it.
#[derive(Debug)]
pub struct Wanting {
    pool: Pool,
    lane: Lane,
}

impl Pool {
    /// Count as a user waiting for `lane` until the guard is dropped.
    pub fn want(&self, lane: Lane) -> Wanting {
        lock(&self.0.state).users_waiting[slot(lane)] += 1;
        Wanting {
            pool: self.clone(),
            lane,
        }
    }
}

impl Drop for Wanting {
    fn drop(&mut self) {
        lock(&self.pool.0.state).users_waiting[slot(self.lane)] -= 1;
        self.pool.0.freed.notify_all();
    }
}

/// One running task's slot; dropping it frees the slot.
#[derive(Debug)]
pub struct Permit {
    pool: Pool,
    lane: Lane,
}

impl Permit {
    pub fn lane(&self) -> Lane {
        self.lane
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = lock(&self.pool.0.state);
        state.running -= 1;
        if self.lane == Lane::Media {
            state.media -= 1;
        }
        drop(state);
        self.pool.0.freed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::CancelToken;
    use crate::registry::Backend;
    use std::num::NonZeroUsize;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn jobs_parse() {
        assert_eq!("auto".parse::<Jobs>(), Ok(Jobs::Auto));
        assert_eq!(
            "3".parse::<Jobs>(),
            Ok(Jobs::Fixed(NonZeroUsize::new(3).unwrap()))
        );
        for bad in ["0", "-1", "two", ""] {
            assert_eq!(
                bad.parse::<Jobs>(),
                Err("takes a number of at least 1, or auto".to_string())
            );
        }
    }

    #[test]
    fn video_and_audio_take_the_media_lane() {
        assert_eq!(Lane::of(Backend::Ffmpeg), Lane::Media);
        for b in [Backend::Image, Backend::Pdf, Backend::Archive] {
            assert_eq!(Lane::of(b), Lane::Cpu);
        }
    }

    #[test]
    fn limits_cap_every_lane() {
        let pool = Pool::new(Limits { total: 3, media: 1 });
        let a = pool.try_acquire(Lane::Media).unwrap();
        assert!(pool.try_acquire(Lane::Media).is_none());
        let b = pool.try_acquire(Lane::Cpu).unwrap();
        let c = pool.try_acquire(Lane::Cpu).unwrap();
        assert!(pool.try_acquire(Lane::Cpu).is_none());
        assert_eq!(pool.running(), 3);
        drop(a);
        assert!(pool.try_acquire(Lane::Cpu).is_some());
        drop((b, c));
        assert_eq!(pool.running(), 0);
    }

    #[test]
    fn auto_is_the_performance_cores() {
        assert_eq!(
            Limits::auto(),
            Limits {
                total: performance_cores(),
                media: 1
            }
        );
        assert!(performance_cores() >= 1);
        assert_eq!(
            Limits::of(Jobs::Fixed(NonZeroUsize::new(5).unwrap())),
            Limits { total: 5, media: 1 }
        );
        assert_eq!(Limits::serial(), Limits { total: 1, media: 1 });
    }

    #[test]
    fn a_waiting_acquire_returns_none_once_cancelled() {
        let pool = Pool::new(Limits::serial());
        let held = pool.try_acquire(Lane::Cpu).unwrap();
        let cancel = CancelToken::new();
        let waiter = {
            let (pool, cancel) = (pool.clone(), cancel.clone());
            std::thread::spawn(move || pool.acquire(Lane::Cpu, Priority::User, &cancel).is_some())
        };
        std::thread::sleep(Duration::from_millis(100));
        cancel.cancel();
        let started = std::time::Instant::now();
        assert!(!waiter.join().unwrap());
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(held);
    }

    #[test]
    fn user_work_goes_before_waiting_background_work() {
        let pool = Pool::new(Limits::serial());
        let held = pool.try_acquire(Lane::Cpu).unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let spawn = |priority: Priority, name: &'static str| {
            let (pool, order) = (pool.clone(), Arc::clone(&order));
            std::thread::spawn(move || {
                let permit = pool.acquire(Lane::Cpu, priority, &CancelToken::new());
                order.lock().unwrap().push(name);
                std::thread::sleep(Duration::from_millis(50));
                drop(permit);
            })
        };
        let background = spawn(Priority::Background, "background");
        std::thread::sleep(Duration::from_millis(100));
        let user = spawn(Priority::User, "user");
        std::thread::sleep(Duration::from_millis(100));
        drop(held);
        background.join().unwrap();
        user.join().unwrap();
        assert_eq!(*order.lock().unwrap(), vec!["user", "background"]);
    }

    /// A thread waiting for a Background slot in `lane`; it reports when it
    /// gets one, then holds it until `release` fires.
    fn background(
        pool: &Pool,
        lane: Lane,
    ) -> (
        std::sync::mpsc::Receiver<()>,
        CancelToken,
        std::thread::JoinHandle<()>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let release = CancelToken::new();
        let (pool, r) = (pool.clone(), release.clone());
        let handle = std::thread::spawn(move || {
            let permit = pool.acquire(lane, Priority::Background, &CancelToken::new());
            let _ = tx.send(());
            while !r.is_cancelled() {
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(permit);
        });
        (rx, release, handle)
    }

    #[test]
    fn a_registered_want_holds_background_back() {
        let pool = Pool::new(Limits::serial());
        let held = pool.try_acquire(Lane::Cpu).unwrap();
        let (got, release, bg) = background(&pool, Lane::Cpu);
        std::thread::sleep(Duration::from_millis(100));
        let want = pool.want(Lane::Cpu);
        drop(held);
        assert!(
            got.recv_timeout(Duration::from_millis(200)).is_err(),
            "background work took the slot a user wanted"
        );
        let user = pool.try_acquire(Lane::Cpu).unwrap();
        drop(want);
        drop(user);
        got.recv_timeout(Duration::from_secs(1)).unwrap();
        release.cancel();
        bg.join().unwrap();
    }

    #[test]
    fn background_media_yields_to_a_user_wanting_cpu() {
        let pool = Pool::new(Limits { total: 1, media: 1 });
        let held = pool.try_acquire(Lane::Cpu).unwrap();
        let (got, release, bg) = background(&pool, Lane::Media);
        std::thread::sleep(Duration::from_millis(100));
        let want = pool.want(Lane::Cpu);
        drop(held);
        assert!(
            got.recv_timeout(Duration::from_millis(200)).is_err(),
            "background video took the slot a user's image wanted"
        );
        assert_eq!(pool.running(), 0);
        drop(want);
        got.recv_timeout(Duration::from_secs(1)).unwrap();
        release.cancel();
        bg.join().unwrap();
    }

    #[test]
    fn background_goes_when_the_wanted_lane_is_full() {
        let pool = Pool::new(Limits { total: 3, media: 1 });
        let _video = pool.try_acquire(Lane::Media).unwrap();
        let _want = pool.want(Lane::Media);
        let (got, release, bg) = background(&pool, Lane::Cpu);
        got.recv_timeout(Duration::from_millis(200))
            .expect("background image work waited for a user whose lane is full");
        release.cancel();
        bg.join().unwrap();
    }

    #[cfg(feature = "vips")]
    #[test]
    fn a_parallel_pool_sets_libvips_to_one_thread() {
        let _p = Pool::new(Limits { total: 4, media: 1 });
        assert_eq!(rs_vips::Vips::concurrency_get(), 1);
    }
}
