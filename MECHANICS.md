# Mechanics sheet

The original program (`ETS6.exe`) could not be decompiled in this round: the project now runs on
Linux and the program's Windows install folder is not reachable from here. The values below come
from the published KNX protocol (as already used by the download code in `knx.rs`) or are marked
**estimated**. Keep `mechanics.json` in step with this file.

## Diagnostics (Monitors › Diagnostics)

The pages follow the original's Monitors panel tree (reference picture "Monitors"): Device Info,
Individual Addresses › Programming Mode, Individual Address Check, Line Scan.

| Value | Source |
|---|---|
| Device descriptor read: A_DeviceDescriptor_Read (APCI 0x300, type 0), answer 0x340 with the 2-byte mask version | KNX standard |
| Mask version: upper nibble = medium (0 TP1, 1 PL110, 2 RF, 5 IP), lower 12 bits = device type (0x010–0x013 System 1, 0x020–0x025 System 2, 0x701/0x705 System 7, 0x7B0 System B, 0x910/0x91A couplers) | KNX standard |
| Identity from the device object (object 0): manufacturer PID 12, serial number PID 11, order info PID 15, firmware revision PID 9, hardware type PID 78, programming mode PID 54 | KNX standard |
| Application program id: object 4, PID 13 (manufacturer 2 bytes, number 2 bytes, version 1 byte) | KNX standard |
| Load states: PID 5 of object 1 (group address table), 2 (association table), 4 (application); 0 unloaded, 1 loaded, 2 loading, 3 error, 4 unloading, 5 completing | KNX standard |
| System 1/2 devices: programming mode in memory 0x0060 bit 0 (bit 7 parity, written as 0x81 / 0x00); application id at memory 0x0104 (4 bytes) | KNX standard |
| Programming mode search: broadcast A_IndividualAddress_Read, collect answers (0x140) for 2.5 s | `read_prog_mode` in knx.rs (existing) |
| Programming mode repeat interval while "Start" is on: 3 s after each 2.5 s search | **estimated** |
| Address probe: T_Connect + A_DeviceDescriptor_Read; a T_ACK or descriptor answer within 0.6 s means a device exists, then up to 1.5 s more for the mask | **estimated** (timeouts) |
| Line scan: addresses 0–255 of the line, one probe each, in order; can be stopped | KNX standard (address range); order **estimated** |
| Switching programming mode on/off: property write PID 54 (System B and newer), else memory write 0x0060; confirmed by a programming mode search | KNX standard |

## Simulation (no hardware)

| Value | Source |
|---|---|
| A project device answers at its address once its address is programmed; before that it sits at 15.15.255 (factory address) | KNX standard (factory address); behaviour **estimated** |
| Simulated mask version: 07B0 (System B, TP1); 091A for couplers and routers | **estimated** |
| Simulated line scan speed: 64 addresses per second | **estimated** |
| Simulated device info / check / switch durations: 1.2 s / 0.8 s / 0.6 s | **estimated** |
| Simulated load states follow the device's download marks (application loaded when program and parameters are downloaded, tables when group addresses are) | **estimated** |
