use std::io::{File, IoResult};
use std::mem;
use tox::core::*;
use self::FileState::*;

pub const QUEUE_ACTIVE_LEN: uint = 20;
pub const INCOMPLETE_DIR: &'static str = "incomplete";

#[deriving(Clone,PartialEq)]
pub enum FileState {
    Active,
    Paused,
    Waiting,
    Broken,
}

pub struct FileReceiver {
    fnum: i32,
    fid: u8,
    file: File,
    state: FileState,
    received: u64,
}

impl FileReceiver {
    pub fn new(fnum: i32, fid: u8, name: &str) -> IoResult<FileReceiver> {
        let mut path = Path::new(INCOMPLETE_DIR);
        path.push(name);
        let file = try!(File::create(&path));

        Ok(FileReceiver {
            fnum: fnum,
            fid: fid,
            file: file,
            state: Active,
            received: 0,
        })
    }

    pub fn write(&mut self, data: Vec<u8>) -> IoResult<()> {
        try!(self.file.write(&*data));
        self.received += data.len() as u64;
        Ok(())
    }
}

pub struct FileQueue<'a> {
    tox: &'a Tox,
    waiting: Vec<FileReceiver>,
    active: Vec<FileReceiver>,
}

impl<'a> FileQueue<'a> {
    pub fn new(tox: &Tox) -> FileQueue {
        FileQueue {
            tox: tox,
            waiting: Vec::new(),
            active: Vec::with_capacity(QUEUE_ACTIVE_LEN),
        }
    }

    pub fn add(&mut self, fnum: i32, fid: u8, name: Vec<u8>) {
        let fr = match ::std::str::from_utf8(&*name).and_then(|fname| FileReceiver::new(fnum, fid, fname).ok()) {
            Some(fr) => fr,
            None => {
                self.tox.file_send_control(fnum, TransferType::Receiving, fid, ControlType::Kill as u8, Vec::new()).unwrap();
                return
            },
        };
        if self.active.len() < QUEUE_ACTIVE_LEN {
            self.active.push(fr);
            self.tox.file_send_control(fnum, TransferType::Receiving, fid, ControlType::Accept as u8, Vec::new());
        } else {
            self.waiting.push(fr);
        }
    }

    pub fn write(&mut self, fnum: i32, fid: u8, data: Vec<u8>) {
        let pos = self.active.iter_mut().position(|fr| fr.fnum == fnum && fr.fid == fid).unwrap();

        if self.active[pos].write(data).is_err() {
            self.tox.file_send_control(fnum, TransferType::Receiving, fid, ControlType::Kill as u8, Vec::new()).unwrap();
            self.remove(fnum, fid);
        }
    }

    pub fn has_paused(&mut self, fnum: i32, fid: u8) {
        if let Some(i) = self.active.iter().position(|fr| fr.fnum == fnum && fr.fid == fid) {
            self.active[i].state = Waiting;
            self.waiting.push(self.active.remove(i).unwrap());
        }
    }

    pub fn has_resumed(&mut self, fnum: i32, fid: u8) {
        if let Some(i) = self.waiting.iter().position(|fr| fr.fnum == fnum && fr.fid == fid) {
            self.waiting[i].state = Active;
            if self.active.len() < QUEUE_ACTIVE_LEN {
                self.active.push(self.waiting.remove(i).unwrap());
            } else {
                self.tox.file_send_control(fnum, TransferType::Receiving, fid, ControlType::Pause as u8, Vec::new());
            }
        }
    }

    //pub fn pause(&self, fid: u8) {}
    //pub fn resume(){}

    pub fn offline(&mut self, fnum: i32) {
        let active = mem::replace(&mut self.active, Vec::new());
        let (broken, active) = active.partition(|fr| fr.fnum == fnum && fr.state == Active);
        self.active = active;
        self.waiting.extend(broken.into_iter());
        self.waiting.iter_mut().map(|fr| fr.state = Broken).fold((), |_,_| ());
    }

    pub fn online(&mut self, fnum: i32) {
        let tox = self.tox;
        self.waiting.iter_mut().map(|fr| {
            if fr.state == Broken {
                // Ad-hoc solution. Just because rust-tox API sucks.
                let mut received = Vec::new();
                received.write_le_u64(fr.received);
                tox.file_send_control(fnum, TransferType::Receiving, fr.fid, ControlType::ResumeBroken as u8, received);
                fr.state = Waiting;
            }
        }).fold((), |_,_| ());
    }

    pub fn finished(&mut self, fnum: i32, fid: u8) -> String {
        let path = self.active.iter().find(|fr| fr.fnum == fnum && fr.fid == fid).unwrap()
                   .file.path().as_str().unwrap().to_string();
        self.remove(fnum, fid);
        self.tox.file_send_control(fnum, TransferType::Receiving, fid, ControlType::Finished as u8, Vec::new());
        path
    }

    pub fn remove(&mut self, fnum: i32, fid: u8) {
        if let Some(i) = self.active.iter().position(|fr| fr.fnum == fnum && fr.fid == fid) {
            match self.waiting.iter().position(|fr| fr.state == Active) {
                Some(j) => {
                    self.active[i] = self.waiting.remove(j).unwrap();
                    self.tox.file_send_control(self.active[i].fnum,
                        TransferType::Receiving, self.active[i].fid, ControlType::Accept as u8, Vec::new());
                },
                None => {
                    self.active.remove(i).unwrap();
                },
            }
        }
    }
}
