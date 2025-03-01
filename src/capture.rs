use std::ops::Range;

use crate::file_vec::FileVec;
use crate::hybrid_index::HybridIndex;
use crate::usb::{
    PID,
    PacketFields,
    SetupFields,
    Direction,
    DeviceDescriptor,
    Configuration,
    ControlTransfer,
};

use bytemuck_derive::{Pod, Zeroable};
use num_enum::{IntoPrimitive, FromPrimitive};
use num_format::{Locale, ToFormattedString};
use humansize::{FileSize, file_size_opts as options};

#[derive(Clone)]
pub enum Item {
    Transfer(u64),
    Transaction(u64, u64),
    Packet(u64, u64, u64),
}

#[derive(Clone)]
pub enum DeviceItem {
    Device(u64),
    DeviceDescriptor(u64),
    DeviceDescriptorField(u64, u8),
    Configuration(u64, u8),
    ConfigurationDescriptor(u64, u8),
    ConfigurationDescriptorField(u64, u8, u8),
    Interface(u64, u8, u8),
    InterfaceDescriptor(u64, u8, u8),
    InterfaceDescriptorField(u64, u8, u8, u8),
    EndpointDescriptor(u64, u8, u8, u8),
    EndpointDescriptorField(u64, u8, u8, u8, u8),
}

#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Device {
    pub address: u8,
}

bitfield! {
    #[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
    #[repr(C)]
    pub struct Endpoint(u64);
    pub u64, device_id, set_device_id: 51, 0;
    pub u8, device_address, set_device_address: 58, 52;
    pub u8, number, set_number: 63, 59;
}

bitfield! {
    #[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
    #[repr(C)]
    pub struct TransferIndexEntry(u64);
    pub u64, transfer_id, set_transfer_id: 51, 0;
    pub u16, endpoint_id, set_endpoint_id: 62, 52;
    pub u8, _is_start, _set_is_start: 63, 63;
}

impl TransferIndexEntry {
    pub fn is_start(&self) -> bool {
        self._is_start() != 0
    }
    pub fn set_is_start(&mut self, value: bool) {
        self._set_is_start(value as u8)
    }
}

#[derive(Copy, Clone, IntoPrimitive, FromPrimitive, PartialEq)]
#[repr(u8)]
pub enum EndpointState {
    #[default]
    Idle = 0,
    Starting = 1,
    Ongoing = 2,
    Ending = 3,
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
#[repr(u8)]
pub enum EndpointType {
    Control       = 0x00,
    Isochronous   = 0x01,
    Bulk          = 0x02,
    Interrupt     = 0x03,
    #[default]
    Unidentified  = 0x04,
    Framing       = 0x10,
    Invalid       = 0x11,
}

pub struct EndpointTraffic {
    pub transaction_ids: HybridIndex,
    pub transfer_index: HybridIndex,
}

pub struct DeviceData {
    pub device_descriptor: Option<DeviceDescriptor>,
    pub configurations: Vec<Option<Configuration>>,
    pub configuration_id: Option<usize>,
    pub endpoint_types: Vec<EndpointType>,
    pub strings: Vec<Option<Vec<u8>>>,
}

impl DeviceData {
    pub fn endpoint_type(&self, number: usize) -> EndpointType {
        use EndpointType::*;
        match number {
            0 => Control,
            0x10 => Framing,
            0x11 => Invalid,
            _ => self.endpoint_types[number],
        }
    }

    pub fn update_endpoint_types(&mut self) {
        match self.configuration_id {
            Some(id) => match &self.configurations[id] {
                Some(config) => {
                    for iface in &config.interfaces {
                        for ep_desc in &iface.endpoint_descriptors {
                            let number = ep_desc.endpoint_address & 0x0F;
                            let index = number as usize;
                            self.endpoint_types[index] =
                                EndpointType::from(ep_desc.attributes & 0x03);
                        }
                    }
                },
                None => {},
            },
            None => {},
        }
    }
}

pub struct Capture {
    pub item_index: HybridIndex,
    pub packet_index: HybridIndex,
    pub packet_data: FileVec<u8>,
    pub transaction_index: HybridIndex,
    pub transfer_index: FileVec<TransferIndexEntry>,
    pub devices: FileVec<Device>,
    pub device_data: Vec<DeviceData>,
    pub endpoints: FileVec<Endpoint>,
    pub endpoint_traffic: Vec<EndpointTraffic>,
    pub endpoint_states: FileVec<u8>,
    pub endpoint_state_index: HybridIndex,
}

impl Default for Capture {
    fn default() -> Self {
        Capture::new()
    }
}

pub struct Transaction {
    pid: PID,
    packet_id_range: Range<u64>,
    payload_byte_range: Option<Range<u64>>,
}

impl Transaction {
    fn packet_count(&self) -> u64 {
        self.packet_id_range.end - self.packet_id_range.start
    }

    fn payload_size(&self) -> Option<u64> {
        match &self.payload_byte_range {
            Some(range) => Some(range.end - range.start),
            None => None
        }
    }
}

fn get_index_range(index: &mut HybridIndex,
                      length: u64,
                      id: u64) -> Range<u64>
{
    if id + 2 > index.len() {
        let start = index.get(id).unwrap();
        let end = length;
        start..end
    } else {
        let vec = index.get_range(id..(id + 2)).unwrap();
        let start = vec[0];
        let end = vec[1];
        start..end
    }
}

pub fn fmt_count(count: u64) -> String {
    count.to_formatted_string(&Locale::en)
}

pub fn fmt_size(size: u64) -> String {
    size.file_size(options::BINARY).unwrap()
}

pub fn fmt_vec<T>(vec: &FileVec<T>) -> String
    where T: bytemuck::Pod + Default
{
    format!("{} entries, {}", fmt_count(vec.len()), fmt_size(vec.size()))
}

pub fn fmt_index(idx: &HybridIndex) -> String {
    format!("{} values in {} entries, {}",
            fmt_count(idx.len()),
            fmt_count(idx.entry_count()),
            fmt_size(idx.size()))
}

impl Capture {
    pub fn new() -> Self {
        Capture {
            item_index: HybridIndex::new(1).unwrap(),
            packet_index: HybridIndex::new(2).unwrap(),
            packet_data: FileVec::new().unwrap(),
            transaction_index: HybridIndex::new(1).unwrap(),
            transfer_index: FileVec::new().unwrap(),
            devices: FileVec::new().unwrap(),
            device_data: Vec::new(),
            endpoints: FileVec::new().unwrap(),
            endpoint_traffic: Vec::new(),
            endpoint_states: FileVec::new().unwrap(),
            endpoint_state_index: HybridIndex::new(1).unwrap(),
        }
    }

    pub fn print_storage_summary(&self) {
        let mut overhead: u64 =
            self.packet_index.size() +
            self.transaction_index.size() +
            self.transfer_index.size() +
            self.endpoint_states.size() +
            self.endpoint_state_index.size();
        let mut trx_count = 0;
        let mut trx_entries = 0;
        let mut trx_size = 0;
        let mut xfr_count = 0;
        let mut xfr_entries = 0;
        let mut xfr_size = 0;
        for ep_traf in &self.endpoint_traffic {
            trx_count += ep_traf.transaction_ids.len();
            trx_entries += ep_traf.transaction_ids.entry_count();
            trx_size += ep_traf.transaction_ids.size();
            xfr_count += ep_traf.transfer_index.len();
            xfr_entries += ep_traf.transfer_index.entry_count();
            xfr_size += ep_traf.transfer_index.size();
            overhead += trx_size + xfr_size;
        }
        let ratio = (overhead as f32) / (self.packet_data.size() as f32);
        let percentage = ratio * 100.0;
        print!(concat!(
            "Storage summary:\n",
            "  Packet data: {}\n",
            "  Packet index: {}\n",
            "  Transaction index: {}\n",
            "  Transfer index: {}\n",
            "  Endpoint states: {}\n",
            "  Endpoint state index: {}\n",
            "  Endpoint transaction indices: {} values in {} entries, {}\n",
            "  Endpoint transfer indices: {} values in {} entries, {}\n",
            "Total overhead: {:.1}% ({})\n"),
            fmt_size(self.packet_data.size()),
            fmt_index(&self.packet_index),
            fmt_index(&self.transaction_index),
            fmt_vec(&self.transfer_index),
            fmt_vec(&self.endpoint_states),
            fmt_index(&self.endpoint_state_index),
            fmt_count(trx_count), fmt_count(trx_entries), fmt_size(trx_size),
            fmt_count(xfr_count), fmt_count(xfr_entries), fmt_size(xfr_size),
            percentage, fmt_size(overhead),
        )
    }

    pub fn get_item(&mut self, parent: &Option<Item>, index: u64) -> Item {
        match parent {
            None => Item::Transfer(self.item_index.get(index).unwrap()),
            Some(item) => self.get_child(item, index)
        }
    }

    pub fn get_child(&mut self, parent: &Item, index: u64) -> Item {
        use Item::*;
        match parent {
            Transfer(transfer_index_id) =>
                Transaction(*transfer_index_id, {
                    let entry = self.transfer_index.get(*transfer_index_id).unwrap();
                    let endpoint_id = entry.endpoint_id() as usize;
                    let transfer_id = entry.transfer_id();
                    let ep_traf = &mut self.endpoint_traffic[endpoint_id];
                    let offset = ep_traf.transfer_index.get(transfer_id).unwrap();
                    ep_traf.transaction_ids.get(offset + index).unwrap()
                }),
            Transaction(transfer_index_id, transaction_id) =>
                Packet(*transfer_index_id, *transaction_id, {
                    self.transaction_index.get(*transaction_id).unwrap() + index}),
            Packet(..) => panic!("packets do not have children"),
        }
    }

    fn item_range(&mut self, item: &Item) -> Range<u64> {
        use Item::*;
        match item {
            Transfer(transfer_index_id) => {
                let entry = self.transfer_index.get(*transfer_index_id).unwrap();
                let endpoint_id = entry.endpoint_id() as usize;
                let transfer_id = entry.transfer_id();
                let ep_traf = &mut self.endpoint_traffic[endpoint_id];
                get_index_range(&mut ep_traf.transfer_index,
                    ep_traf.transaction_ids.len(), transfer_id)
            },
            Transaction(_, transaction_id) => {
                get_index_range(&mut self.transaction_index,
                    self.packet_index.len(), *transaction_id)
            },
            Packet(.., packet_id) => {
                get_index_range(&mut self.packet_index,
                    self.packet_data.len(), *packet_id)
            },
        }
    }

    pub fn item_count(&mut self, parent: &Option<Item>) -> u64 {
        match parent {
            None => self.item_index.len(),
            Some(item) => self.child_count(item)
        }
    }

    pub fn child_count(&mut self, parent: &Item) -> u64 {
        use Item::*;
        match parent {
            Transfer(id) => {
                let entry = self.transfer_index.get(*id).unwrap();
                if entry.is_start() {
                    let range = self.item_range(parent);
                    range.end - range.start
                } else {
                    0
                }
            },
            Transaction(..) => {
                let range = self.item_range(parent);
                range.end - range.start
            },
            Packet(..) => 0,
        }
    }

    pub fn get_summary(&mut self, item: &Item) -> String {
        use Item::*;
        match item {
            Packet(.., packet_id) => {
                let packet = self.get_packet(*packet_id);
                let pid = PID::from(packet[0]);
                format!("{} packet{}: {:02X?}",
                    pid,
                    match PacketFields::from_packet(&packet) {
                        PacketFields::SOF(sof) => format!(
                            " with frame number {}, CRC {:02X}",
                            sof.frame_number(),
                            sof.crc()),
                        PacketFields::Token(token) => format!(
                            " on {}.{}, CRC {:02X}",
                            token.device_address(),
                            token.endpoint_number(),
                            token.crc()),
                        PacketFields::Data(data) => format!(
                            " with {} data bytes and CRC {:04X}",
                            packet.len() - 3,
                            data.crc),
                        PacketFields::None => "".to_string()
                    },
                    packet)
            },
            Transaction(_, transaction_id) => {
                let transaction = self.get_transaction(transaction_id);
                let count = transaction.packet_count();
                match (transaction.pid, transaction.payload_size()) {
                    (PID::SOF, _) => format!(
                        "{} SOF packets", count),
                    (pid, None) => format!(
                        "{} transaction, {} packets", pid, count),
                    (pid, Some(size)) => format!(
                        "{} transaction, {} packets with {} data bytes",
                        pid, count, size)
                }
            },
            Transfer(transfer_index_id) => {
                let entry = self.transfer_index.get(*transfer_index_id).unwrap();
                let endpoint_id = entry.endpoint_id();
                let endpoint = self.endpoints.get(endpoint_id as u64).unwrap();
                let device_id = endpoint.device_id() as usize;
                let dev_data = &self.device_data[device_id];
                let num = endpoint.number() as usize;
                let ep_type = dev_data.endpoint_type(num);
                if !entry.is_start() {
                    return match ep_type {
                        EndpointType::Invalid =>
                            "End of invalid groups".to_string(),
                        EndpointType::Framing =>
                            "End of SOF groups".to_string(),
                        endpoint_type => format!(
                            "{:?} transfer ending on endpoint {}.{}",
                            endpoint_type, endpoint.device_address(), num)
                    }
                }
                let range = self.item_range(&item);
                let count = range.end - range.start;
                match ep_type {
                    EndpointType::Invalid => format!(
                        "{} invalid groups", count),
                    EndpointType::Framing => format!(
                        "{} SOF groups", count),
                    EndpointType::Control => {
                        let transfer = self.get_control_transfer(
                            endpoint.device_address(), endpoint_id, range);
                        transfer.summary()
                    },
                    endpoint_type => format!(
                        "{:?} transfer with {} transactions on endpoint {}.{}",
                        endpoint_type, count,
                        endpoint.device_address(), endpoint.number())
                }
            }
        }
    }

    pub fn get_connectors(&mut self, item: &Item) -> String {
        use EndpointState::*;
        use Item::*;
        let endpoint_count = self.endpoints.len() as usize;
        const MIN_LEN: usize = " └─".len();
        let string_length = MIN_LEN + endpoint_count;
        let mut connectors = String::with_capacity(string_length);
        let transfer_index_id = match item {
            Transfer(i) | Transaction(i, _) | Packet(i, ..) => i
        };
        let entry = self.transfer_index.get(*transfer_index_id).unwrap();
        let endpoint_id = entry.endpoint_id() as usize;
        let endpoint_state = self.get_endpoint_state(*transfer_index_id);
        let state_length = endpoint_state.len();
        let extended = self.transfer_extended(endpoint_id, *transfer_index_id);
        let ep_traf = &mut self.endpoint_traffic[endpoint_id];
        let last_transaction = match item {
            Transaction(_, transaction_id) | Packet(_, transaction_id, _) => {
                let range = get_index_range(&mut ep_traf.transfer_index,
                    ep_traf.transaction_ids.len(), entry.transfer_id());
                let last_transaction_id =
                    ep_traf.transaction_ids.get(range.end - 1).unwrap();
                *transaction_id == last_transaction_id
            }, _ => false
        };
        let last_packet = match item {
            Packet(_, transaction_id, packet_id) => {
                let range = get_index_range(&mut self.transaction_index,
                    self.packet_index.len(), *transaction_id);
                *packet_id == range.end - 1
            }, _ => false
        };
        let last = last_transaction && !extended;
        let mut thru = false;
        for i in 0..state_length {
            let state = EndpointState::from(endpoint_state[i]);
            let active = state != Idle;
            let on_endpoint = i == endpoint_id;
            thru |= match (item, state, on_endpoint) {
                (Transfer(..), Starting | Ending, _) => true,
                (Transaction(..) | Packet(..), _, true) => on_endpoint,
                _ => false,
            };
            connectors.push(match item {
                Transfer(..) => {
                    match (state, thru) {
                        (Idle,     _    ) => ' ',
                        (Starting, _    ) => '○',
                        (Ongoing,  false) => '│',
                        (Ongoing,  true ) => '┼',
                        (Ending,   _    ) => '└',
                    }
                },
                Transaction(..) => {
                    match (on_endpoint, active, thru, last) {
                        (false, false, false, _    ) => ' ',
                        (false, false, true,  _    ) => '─',
                        (false, true,  false, _    ) => '│',
                        (false, true,  true,  _    ) => '┼',
                        (true,  _,     _,     false) => '├',
                        (true,  _,     _,     true ) => '└',
                    }
                },
                Packet(..) => {
                    match (on_endpoint, active, last) {
                        (false, false, _    ) => ' ',
                        (false, true,  _    ) => '│',
                        (true,  _,     false) => '│',
                        (true,  _,     true ) => ' ',
                    }
                }
            });
        };
        for _ in state_length..endpoint_count {
            connectors.push(match item {
                Transfer(..)    => '─',
                Transaction(..) => '─',
                Packet(..)      => ' ',
            });
        }
        connectors.push_str(
            match (item, last_packet) {
                (Transfer(_), _) if entry.is_start() => "─",
                (Transfer(_), _)                     => "──□ ",
                (Transaction(..), _)                 => "───",
                (Packet(..), false)                  => "    ├──",
                (Packet(..), true)                   => "    └──",
            }
        );
        connectors
    }

    fn transfer_extended(&mut self, endpoint_id: usize, index: u64) -> bool {
        use EndpointState::*;
        let count = self.transfer_index.len();
        if index + 1 >= count {
            return false;
        };
        let state = self.get_endpoint_state(index + 1);
        if endpoint_id >= state.len() {
            false
        } else {
            match EndpointState::from(state[endpoint_id]) {
                Ongoing => true,
                _ => false,
            }
        }
    }

    fn get_endpoint_state(&mut self, index: u64) -> Vec<u8> {
        let range = get_index_range(
            &mut self.endpoint_state_index,
            self.endpoint_states.len(), index);
        self.endpoint_states.get_range(range).unwrap()
    }

    fn get_packet(&mut self, index: u64) -> Vec<u8> {
        let range = get_index_range(&mut self.packet_index,
                                    self.packet_data.len(), index);
        self.packet_data.get_range(range).unwrap()
    }

    fn get_packet_pid(&mut self, index: u64) -> PID {
        let offset = self.packet_index.get(index).unwrap();
        PID::from(self.packet_data.get(offset).unwrap())
    }

    fn get_transaction(&mut self, index: &u64) -> Transaction {
        let packet_id_range = get_index_range(&mut self.transaction_index,
                                              self.packet_index.len(), *index);
        let packet_count = packet_id_range.end - packet_id_range.start;
        let pid = self.get_packet_pid(packet_id_range.start);
        use PID::*;
        let payload_byte_range = match pid {
            IN | OUT if packet_count >= 2 => {
                let data_packet_id = packet_id_range.start + 1;
                let packet_byte_range = get_index_range(
                    &mut self.packet_index,
                    self.packet_data.len(), data_packet_id);
                let pid = self.packet_data.get(packet_byte_range.start).unwrap();
                match PID::from(pid) {
                    DATA0 | DATA1 => Some({
                        packet_byte_range.start + 1 .. packet_byte_range.end - 2
                    }),
                    _ => None
                }
            },
            _ => None
        };
        Transaction {
            pid: pid,
            packet_id_range: packet_id_range,
            payload_byte_range: payload_byte_range,
        }
    }

    fn get_control_transfer(&mut self,
                            address: u8,
                            endpoint_id: u16,
                            range: Range<u64>) -> ControlTransfer
    {
        let ep_traf = &mut self.endpoint_traffic[endpoint_id as usize];
        let transaction_ids =
            ep_traf.transaction_ids.get_range(range).unwrap();
        let setup_transaction_id = transaction_ids[0];
        let setup_packet_id =
            self.transaction_index.get(setup_transaction_id)
                                  .unwrap();
        let data_packet_id = setup_packet_id + 1;
        let data_packet = self.get_packet(data_packet_id);
        let fields = SetupFields::from_data_packet(&data_packet);
        let direction = fields.type_fields.direction();
        let mut data: Vec<u8> = Vec::new();
        for id in transaction_ids {
            let transaction = self.get_transaction(&id);
            match (direction,
                   transaction.pid,
                   transaction.payload_byte_range)
            {
                (Direction::In,  PID::IN,  Some(range)) |
                (Direction::Out, PID::OUT, Some(range)) => {
                    data.extend_from_slice(
                        &self.packet_data.get_range(range).unwrap());
                },
                (..) => {}
            };
        }
        ControlTransfer {
            address: address,
            fields: fields,
            data: data,
        }
    }

    pub fn get_device_item(&mut self, parent: &Option<DeviceItem>, index: u64)
        -> DeviceItem
    {
        match parent {
            None => DeviceItem::Device(index + 1),
            Some(item) => self.device_child(item, index)
        }
    }

    fn device_child(&self, item: &DeviceItem, index: u64) -> DeviceItem {
        use DeviceItem::*;
        match item {
            Device(dev) => match index {
                0 => DeviceDescriptor(*dev),
                conf => Configuration(*dev, conf as u8),
            },
            DeviceDescriptor(dev) =>
                DeviceDescriptorField(*dev, index as u8),
            Configuration(dev, conf) => match index {
                0 => ConfigurationDescriptor(*dev, *conf),
                n => Interface(*dev, *conf, (n - 1).try_into().unwrap()),
            },
            ConfigurationDescriptor(dev, conf) =>
                ConfigurationDescriptorField(*dev, *conf, index as u8),
            Interface(dev, conf, iface) => match index {
                0 => InterfaceDescriptor(*dev, *conf, *iface),
                n => EndpointDescriptor(*dev, *conf, *iface,
                                        (n - 1).try_into().unwrap())
            },
            InterfaceDescriptor(dev, conf, iface) =>
                InterfaceDescriptorField(*dev, *conf, *iface, index as u8),
            EndpointDescriptor(dev, conf, iface, ep) =>
                 EndpointDescriptorField(*dev, *conf, *iface,
                                         *ep, index as u8),
            _ => panic!("Item does not have children")
        }
    }

    pub fn device_item_count(&mut self, parent: &Option<DeviceItem>) -> u64 {
        match parent {
            None => (self.device_data.len() - 1) as u64,
            Some(item) => self.device_child_count(item),
        }
    }

    fn device_child_count(&self, item: &DeviceItem) -> u64 {
        use DeviceItem::*;
        let data = &self.device_data;
        (match item {
            Device(dev) =>
                data[*dev as usize].configurations.len(),
            DeviceDescriptor(dev) =>
                match data[*dev as usize].device_descriptor {
                    Some(_) => 13,
                    None => 0,
                },
            Configuration(dev, conf) =>
                match data[*dev as usize]
                    .configurations[*conf as usize].as_ref()
                {
                    Some(conf) => 1 + conf.interfaces.len(),
                    None => 0
                },
            ConfigurationDescriptor(dev, conf) =>
                match data[*dev as usize]
                    .configurations[*conf as usize]
                {
                    Some(_) => 8,
                    None => 0
                },
            Interface(dev, conf, iface) =>
                match data[*dev as usize]
                    .configurations[*conf as usize].as_ref()
                {
                    Some(conf) => 1 + conf.interfaces[*iface as usize]
                        .endpoint_descriptors.len(),
                    None => 0
                },
            InterfaceDescriptor(..) => 9,
            EndpointDescriptor(..) => 6,
            _ => 0
        }) as u64
    }

    pub fn get_device_summary(&mut self, item: &DeviceItem) -> String {
        use DeviceItem::*;
        match item {
            Device(dev) => {
                let data = &self.device_data[*dev as usize];
                let device = self.devices.get(*dev).unwrap();
                format!("Device {}: {}", device.address,
                    match data.device_descriptor {
                        Some(descriptor) => format!(
                            "{:04X}:{:04X}",
                            descriptor.vendor_id,
                            descriptor.product_id
                        ),
                        None => format!("Unknown"),
                    }
                )
            },
            DeviceDescriptor(dev) => {
                let data = &self.device_data[*dev as usize];
                match data.device_descriptor {
                    Some(_) => "Device descriptor",
                    None => "No device descriptor"
                }.to_string()
            },
            DeviceDescriptorField(dev, field) => {
                let data = &self.device_data[*dev as usize];
                let desc = data.device_descriptor.unwrap();
                desc.field_text(*field, &data.strings)
            },
            Configuration(_, conf) => format!(
                "Configuration {}", conf),
            ConfigurationDescriptor(..) =>
                "Configuration descriptor".to_string(),
            ConfigurationDescriptorField(dev, conf, field) => {
                let data = &self.device_data[*dev as usize];
                let config = &data.configurations[*conf as usize];
                let config = config.as_ref().unwrap();
                config.descriptor.field_text(*field, &data.strings)
            },
            Interface(_, _, iface) => format!(
                "Interface {}", iface),
            InterfaceDescriptor(..) =>
                "Interface descriptor".to_string(),
            InterfaceDescriptorField(dev, conf, iface, field) => {
                let data = &self.device_data[*dev as usize];
                let config = &data.configurations[*conf as usize];
                let config = config.as_ref().unwrap();
                let iface = &config.interfaces[*iface as usize];
                iface.descriptor.field_text(*field, &data.strings)
            },
            EndpointDescriptor(dev, conf, iface, ep) => {
                let data = &self.device_data[*dev as usize];
                let config = &data.configurations[*conf as usize];
                let config = config.as_ref().unwrap();
                let iface = &config.interfaces[*iface as usize];
                let desc = iface.endpoint_descriptors[*ep as usize];
                format!("Endpoint {} {}",
                    desc.endpoint_address & 0x7F,
                    if desc.endpoint_address & 0x80 != 0 {"IN"} else {"OUT"}
                )
            },
            EndpointDescriptorField(dev, conf, iface, ep, field) => {
                let data = &self.device_data[*dev as usize];
                let config = &data.configurations[*conf as usize];
                let config = config.as_ref().unwrap();
                let iface = &config.interfaces[*iface as usize];
                let desc = iface.endpoint_descriptors[*ep as usize];
                desc.field_text(*field)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{BufReader, BufWriter, BufRead, Write};
    use crate::decoder::Decoder;

    fn write_item(cap: &mut Capture, item: &Item, depth: u8,
                  writer: &mut dyn Write)
    {
        let summary = cap.get_summary(&item);
        for _ in 0..depth {
            writer.write(b" ").unwrap();
        }
        writer.write(summary.as_bytes()).unwrap();
        writer.write(b"\n").unwrap();
        let num_children = cap.child_count(&item);
        for child_id in 0..num_children {
            let child = cap.get_child(&item, child_id);
            write_item(cap, &child, depth + 1, writer);
        }
    }

    #[test]
    fn test_captures() {
        let test_dir = "./tests/";
        for result in std::fs::read_dir(test_dir).unwrap() {
            let entry = result.unwrap();
            if entry.file_type().unwrap().is_dir() {
                let path = entry.path();
                let mut cap_path = path.clone();
                let mut ref_path = path.clone();
                let mut out_path = path.clone();
                cap_path.push("capture.pcap");
                ref_path.push("reference.txt");
                out_path.push("output.txt");
                {
                    let mut pcap = pcap::Capture::from_file(cap_path).unwrap();
                    let mut cap = Capture::new();
                    let mut decoder = Decoder::new(&mut cap);
                    while let Ok(packet) = pcap.next() {
                        decoder.handle_raw_packet(&packet);
                    }
                    let out_file = File::create(out_path.clone()).unwrap();
                    let mut out_writer = BufWriter::new(out_file);
                    let num_items = cap.item_index.len();
                    for item_id in 0 .. num_items {
                        let item = cap.get_item(&None, item_id);
                        write_item(&mut cap, &item, 0, &mut out_writer);
                    }
                }
                let ref_file = File::open(ref_path).unwrap();
                let out_file = File::open(out_path.clone()).unwrap();
                let ref_reader = BufReader::new(ref_file);
                let out_reader = BufReader::new(out_file);
                let mut out_lines = out_reader.lines();
                for line in ref_reader.lines() {
                    let expected = line.unwrap();
                    let actual = out_lines.next().unwrap().unwrap();
                    assert!(actual == expected);
                }
            }
        }
    }
}

