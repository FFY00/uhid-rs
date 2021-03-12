// SPDX-License-Identifier: MIT

use std::convert::TryInto;

use std::fs::{File, OpenOptions};
use std::os::unix::io::RawFd;
use std::io::Write;

use serde::{Serialize, Deserialize};
use serde_big_array::big_array;

big_array!{BigArray;}

const HID_MAX_DESCRIPTOR_SIZE: usize = 4096;

enum Bus {
    PCI,
    ISAPNP,
    USB,
    HIL,
    BLUETOOTH,
    VIRTUAL,
}

enum EventType {
    __LegacyCreate,
    Destroy,
    Start,
    Stop,
    Open,
    Close,
    Output,
    __LoegacyOutputEv,
    __LegacyInput,
    GetReport,
    GetReportReply,
    Create2,
    Input2,
    SetReport,
    SetReportReply,
}

#[derive(Serialize, Debug)]
struct Create2Req {
    #[serde(with = "BigArray")]
    name: [u8; 128],
    #[serde(with = "BigArray")]
    phys: [u8; 64],
    #[serde(with = "BigArray")]
    uniq: [u8; 64],
    rd_size: u16,
    bus: u16,
    vendor: u32,
    product: u32,
    version: u32,
    country: u32,
    #[serde(with = "BigArray")]
    rd_data: [u8; HID_MAX_DESCRIPTOR_SIZE],
}

pub struct Device {
    uhid_fd: File,
    created: bool,
}

impl Device {
    pub fn new() -> Result<Self, String> {
        Ok(Device {
            uhid_fd: match OpenOptions::new().read(true).write(true).open("/dev/uhid") {
                Ok(f) => f,
                Err(e) => return Err(format!("failed to open the UHID file descriptor ({})", e)),
            },
            created: false,
        })
    }

    fn event(event_type: EventType, mut data: Option<Vec<u8>>) -> Vec<u8> {
        /* build event manually as serde/bincode does not support unions,
           and so doesn't let us make a struct uhid_event */
        let event_type_id = event_type as u32;
        let mut event = bincode::serialize::<u32>(&event_type_id).unwrap();
        match data {
            Some(mut data_vec) => event.append(&mut data_vec),
            None => (),
        };
        return event;
    }

    pub fn create(&mut self, vid: u32, pid: u32, name: &str, rdesc: &[u8], bus: Option<u16>) -> Result<(), String> {
        if self.created {
            return Err("device already created".to_string());
        }
        self.created = true;

        let name_bytes = name.as_bytes();

        if name_bytes.len() > 128 {
            return Err(format!("invalid name length: {} (max: 128)", name_bytes.len()));
        }
        if rdesc.len() > 128 {
            return Err(format!("invalid report descriptor length: {} (max: {})", name.len(), HID_MAX_DESCRIPTOR_SIZE));
        }

        let mut create_req = Create2Req {
            name: [0; 128],
            phys: [0; 64],
            uniq: [0; 64],
            rd_size: rdesc.len() as u16,
            bus: bus.unwrap_or(Bus::USB as u16),
            vendor: vid,
            product: pid,
            version: 0,
            country: 0,
            rd_data: [0; HID_MAX_DESCRIPTOR_SIZE],
        };

        /* populate the name and report descriptor data - this was the only way I found to do this */
        create_req.name[..name_bytes.len()].clone_from_slice(name_bytes);
        create_req.rd_data[..rdesc.len()].clone_from_slice(rdesc);

        let req_vec: Vec<u8> = Self::event(
            EventType::Create2,
            Some(bincode::serialize(&create_req).unwrap()),
        );

        match self.uhid_fd.write(&req_vec) {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("failed to send event ({})", e)),
        }
    }

    pub fn destroy(&mut self) -> Result<(), String> {
        self.created = false;

        match self.uhid_fd.write(&Self::event(EventType::Destroy, None)) {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("failed to send event ({})", e)),
        }
    }
}

pub struct EpollDevice {
    uhid_dev: Device,
    epoll_fd: RawFd,
}

impl EpollDevice {
    pub fn new() -> Result<Self, String> {
        Ok(EpollDevice {
            uhid_dev: Device::new()?,
            epoll_fd: match epoll::create(false) {
                Ok(fd) => fd,
                Err(e) => return Err(format!("failed to open the epoll file descriptor ({})", e)),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUSE_RDEC: [u8; 55] = [
        0x05, 0x01,  // Usage Page (Generic Desktop)        0
        0x09, 0x02,  // Usage (Mouse)                       2
        0xa1, 0x01,  // Collection (Application)            4
        0x09, 0x02,  // .Usage (Mouse)                      6
        0xa1, 0x02,  // .Collection (Logical)               8
        0x09, 0x01,  // ..Usage (Pointer)                   10
        0xa1, 0x00,  // ..Collection (Physical)             12
        0x05, 0x09,  // ...Usage Page (Button)              14
        0x19, 0x01,  // ...Usage Minimum (1)                16
        0x29, 0x03,  // ...Usage Maximum (3)                18
        0x15, 0x00,  // ...Logical Minimum (0)              20
        0x25, 0x01,  // ...Logical Maximum (1)              22
        0x75, 0x01,  // ...Report Size (1)                  24
        0x95, 0x03,  // ...Report Count (3)                 26
        0x81, 0x02,  // ...Input (Data,Var,Abs)             28
        0x75, 0x05,  // ...Report Size (5)                  30
        0x95, 0x01,  // ...Report Count (1)                 32
        0x81, 0x03,  // ...Input (Cnst,Var,Abs)             34
        0x05, 0x01,  // ...Usage Page (Generic Desktop)     36
        0x09, 0x30,  // ...Usage (X)                        38
        0x09, 0x31,  // ...Usage (Y)                        40
        0x15, 0x81,  // ...Logical Minimum (-127)           42
        0x25, 0x7f,  // ...Logical Maximum (127)            44
        0x75, 0x08,  // ...Report Size (8)                  46
        0x95, 0x02,  // ...Report Count (2)                 48
        0x81, 0x06,  // ...Input (Data,Var,Rel)             50
        0xc0,        // ..End Collection                    52
        0xc0,        // .End Collection                     53
        0xc0,        // End Collection                      54
    ];

    #[test]
    fn create() {
        assert_eq!(2 + 2, 4);

        let mut dev = Device::new().unwrap();
        dev.create(
            0x1234,
            0x4321,
            "my rust UHID device!",
            &MOUSE_RDEC,
            None,
        ).unwrap();
    }
}
