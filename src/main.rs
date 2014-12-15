#![feature(globs,phase)]
#![feature(slicing_syntax)]

extern crate tox;
//extern crate debug;
#[phase(plugin, link)] extern crate log;

use tox::core::*;
use std::io::stdin;
use std::io::{File, Truncate, Write, FileNotFound};
use std::io::Timer;
use std::time::{Duration};
use std::os::args;
use std::comm::channel;
use queue::*;

mod queue;

const MAX_FILE_SIZE: u64 = 240 * 1024 * 1024;

const BOOTSTRAP_IP: &'static str = "192.254.75.98";
const BOOTSTRAP_PORT: u16 = 33445;
const BOOTSTRAP_KEY: &'static str = "951C88B7E75C867418ACDB5D273821372BB5BD652740BCDF623A4FA293E75D2F";
const BOT_NAME: &'static str = "filebot";

fn usage(prog: &str) {
    println!("usage: {} dir [tox_save]", prog);
}

fn save(tox: &Tox, path: Option<&Path>, tock: &Receiver<()>) {
    if path.is_none() { return };

    match tock.try_recv() {
        Ok(_) => {
            let mut out = File::open_mode(path.unwrap(), Truncate, Write).unwrap();
            match out.write(&*tox.save()) {
                Ok(_) => (),
                Err(e) => {
                    error!("cannot save tox data: {}", e)
                },
            }
        },
        Err(_) => ()
    }
}

fn main() {
    let tox = Tox::new(ToxOptions::new()).unwrap();
    tox.set_name(BOT_NAME.to_string()).unwrap();

    // Since i/o is blocking and tox doesn't implement Clone,
    // separate task is used to read commands and send them through channel
    let (cmd_tx, cmd_rx) = channel();
    // not sure if it's the best way to terminate a task
    let (exit_tx, exit_rx) = channel::<bool>();
    spawn(proc() {
        let mut stdin = stdin();
        loop {
            if let Ok(_) = exit_rx.try_recv() {
                break;
            }
            if let Ok(line) = stdin.read_line() {
                cmd_tx.send(line);
            }
            std::io::timer::sleep(Duration::milliseconds(100));
        }
    });

    let args = args();
    let path;   // pls, make a more elegant way to follow lifetimes
    let tox_save: Option<&Path>;
    match &*args {
        [_] => {
            tox_save = None;
        },
        [_, ref tf] => {
            path = Path::new(tf);
            tox_save = Some(&path);

            match File::open(tox_save.unwrap()) {
                Ok(mut f) => match tox.load(f.read_to_end().unwrap()) {
                    Ok(_) => (),
                    Err(_) => error!("tox_save loaded with error"),
                },
                Err(e) => {
                    if e.kind != FileNotFound {
                        panic!("io error: {}", e);
                    }
                }
            }
        },
        _ => {
            usage(&*args[0]);
            return;
        }
    }

    let bootstrap_key = from_str(BOOTSTRAP_KEY).unwrap();
    tox.bootstrap_from_address(BOOTSTRAP_IP.to_string(), BOOTSTRAP_PORT, box bootstrap_key).unwrap();

    println!("info Bot key: {}", tox.get_address());

    let mut fqueue = FileQueue::new(&tox);

    let mut bomb = Timer::new().unwrap();   // Oh my God! JC! A bomb!
    let tick = bomb.periodic(Duration::milliseconds(500));
    loop {
        for ev in tox.events() {
            match ev {
                FriendRequest(id, _) => {
                    tox.add_friend_norequest(id).unwrap();
                },
                /*FriendMessage(fnum, msg) => {
                    match msg[] {
                        "invite" => tox.invite_friend(fnum, gchat).unwrap(),
                        _ => ()
                    }

                },*/
                FileSendRequest(fnum, fid, fsize, fname) => {
                    if fsize > MAX_FILE_SIZE {
                        tox.file_send_control(fnum, TransferType::Receiving, fid, ControlType::Kill as u8, Vec::new()).unwrap();
                        tox.send_message(fnum, "File is too big, max allowed size is 240 MiB".to_string());
                    } else {
                        fqueue.add(fnum, fid, fname);
                    }
                    // avoid saving tox data on file events
                    continue;
                },
                FileData(fnum, fid, data) => {
                    fqueue.write(fnum, fid, data);
                    continue;
                },
                FileControl(fnum, TransferType::Receiving, fid, ControlType::Finished, _) => {
                    let name = fqueue.finished(fnum, fid);
                    println!("finished {}", name);
                    continue;
                }
                FileControl(fnum, TransferType::Receiving, fid, ControlType::Kill, _) => {
                    fqueue.remove(fnum, fid);
                    continue;
                }
                FileControl(fnum, TransferType::Receiving, fid, ControlType::Pause, _) => {
                    fqueue.has_paused(fnum, fid);
                    continue;
                }
                FileControl(fnum, TransferType::Receiving, fid, ControlType::Accept, _) => {
                    fqueue.has_resumed(fnum, fid);
                    continue;
                }
                ConnectionStatusVar(fnum, ConnectionStatus::Offline) => {
                    println!("offline {}", fnum);
                    fqueue = fqueue.offline(fnum);
                },
                ConnectionStatusVar(fnum, ConnectionStatus::Online) => {
                    println!("online {}", fnum);
                    fqueue.online(fnum);
                },

                // GroupInvite(id, ref addr) if id == groupbot_id => {
                //     tox.join_groupchat(id, addr.clone()).unwrap();
                //     println!("invited to group");
                // },
                _ => { }
            };

            save(&tox, tox_save, &tick);
        }

        if let Ok(line) = cmd_rx.try_recv() {
            let mut lnit = line.trim_chars('\n').split(' ');
            match lnit.next() {
                Some("status") => tox.set_status_message(lnit.collect::<Vec<&str>>().connect(" ")).unwrap(),
                Some("kill") => {
                    exit_tx.send(true);
                    return;
                },
                _ => {},
            }
        }

        std::io::timer::sleep(Duration::milliseconds(50));
    }
}
