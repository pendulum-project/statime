## Global datasets

Default dataset:
- This dataset says things about the entire runtime instance
- Static:
  - Clock Identity
    - Usually just the extended mac address of the computer
  - Number ports:
    - The count of ports that are registered in the runtime. For ordinary clocks this is 1
- Dynamic:
  - Clock quality:
    - Just ask the clock type
- Config:
  - Priority 1
  - Priority 2
  - Domain Number
  - Slave Only
  - SDO ID
    - This must all be configured by the user
- Optional
  - These can probably be skipped for now

Current dataset:
- This dataset says things about the time that we are synchronizing
- All these values result from selecting the master clock and doing time synchronization with it.

Parent dataset:
- Describes the master we are following

Time properties Dataset:
- Describes the properties of the time we are synchronizing to

## Port datasets

Port Dataset:
- A port is basically a network interface, the data of this dataset and some statemachines running
- This dataset describes all instance variables of the port
- In principle we can have a port object that just contains the dataset variables as field variables
- Static:
  - Port identity
    - Based on the (non-changing) index of the port in the runtime
- Dynamic:
  - Either state machine state or data about the messages
- Config:
  - Sets the time intervals, protocol versions and delay mechanism

```rust
struct Runtime {
    clock: impl ClockInterface, // -> Dynamic portion (clock quality) of the DefaultDS
    ports: Vec<Port>, // -> Static portion (number ports) of the DefaultDS
    data: RuntimeData,
}

struct RuntimeData {
    // Static portion of the DefaultDS
    clock_identity: ClockIdentity,
    // Config portion of the DefaultDS
    config: RuntimeConfig,
    // CurrentDS + TimePropertiesDS
    time_data: TimeData,
    // ParentDS
    parent_data: ParentDS,
}

struct Port {
    network_interface: impl NetworkInterface,
    // Static portion of the PortDS
    identity: PortIdentity,
    // Dynamic portion of the PortDS
    state: PortState,
    log_min_delay_req_interval: i8,
    mean_link_delay: TimeInterval,
    // Config portion of the PortDS
    config: PortConfig,
}

struct RuntimeConfig {
    priority1: u8,
    priority2: u8,
    domain_number: u8,
    slave_only: bool,
    sdo_id: u16,
}

// Config portion of the PortDS
struct PortConfig {
    log_announce_interval: i8,
    announce_receipt_timeout: i8,
    log_sync_interval: i8,
    delay_mechanism: DelayMechanism,
    log_min_pdelay_req_interval: i8,
    version_number: u8,
    minor_version_number: u8,
    delay_asymmetry: TimeInterval,
}

```