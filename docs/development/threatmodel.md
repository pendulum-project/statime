# Threat model

This document a threat model, based on the methodology presented by Eleanor
Saitta, that we as developers use as a guide in our development process. It may
not contain all the context needed to fully understand it, if clarifications are
needed please ask us.

The used methodology is entirely manual, but is derived from
[Trike](https://www.octotrike.org/).

## Actors, Assets & Actions

### Actors

We model the following actors:

- System Admin: Administrator of the system running statime
- System User: Non-administrator user of the system running statime
- AML: PTP nodes on the Acceptable master list. This includes all ptp nodes if no acceptable master list is configured.
- Anonymous: PTP nodes not on the Acceptable master list

### Assets

We model the following assets:

- Clock: The system clock
- Configuration: The configuration of statime.
- Port state: The state of the individual PTP ports in the statime instance

### Actions

<table>
    <tr>
        <th></th>
        <th colspan=2>Clock</th>
        <th colspan=2>Configuration</th>
        <th colspan=2>Port state</th>
    </tr>
    <tr>
        <th rowspan=2>System admin</th>
        <td>Create - N/A</td>
        <td bgcolor="green">Read - Always</td>
        <td bgcolor="green">Create - Always</td>
        <td bgcolor="green">Read - Always</td>
        <td>Create - N/A</td>
        <td bgcolor="green">Read - Always</td>
    </tr>
    <tr>
        <td bgcolor="green">Update - Always</td>
        <td>Delete - N/A</td>
        <td bgcolor="green">Update - Always</td>
        <td>Delete - N/A</td>
        <td bgcolor="green">Update - Always*</td>
        <td>Delete - N/A</td>
    </tr>
    <tr>
        <th rowspan=2>System User</th>
        <td>Create - N/A</td>
        <td bgcolor="green">Read - Always</td>
        <td bgcolor="red">Create - Never</td>
        <td bgcolor="orange">Read - Sometimes</td>
        <td>Create - N/A</td>
        <td bgcolor="orange">Read - Sometimes</td>
    </tr>
    <tr>
        <td bgcolor="red">Update - Never</td>
        <td>Delete - N/A</td>
        <td bgcolor="red">Update - Never</td>
        <td>Delete - N/A</td>
        <td bgcolor="red">Update - Never</td>
        <td>Delete - N/A</td>
    </tr>
    <tr>
        <th rowspan=2>AML</th>
        <td>Create - N/A</td>
        <td bgcolor="orange">Read - Sometimes</td>
        <td bgcolor="red">Create - Never</td>
        <td bgcolor="orange">Read - Sometimes</td>
        <td>Create - N/A</td>
        <td bgcolor="orange">Read - Sometimes</td>
    </tr>
    <tr>
        <td bgcolor="orange">Update - Sometimes</td>
        <td>Delete - N/A</td>
        <td bgcolor="red">Update - Never</td>
        <td>Delete - N/A</td>
        <td bgcolor="orange">Update - Sometimes</td>
        <td>Delete - N/A</td>
    </tr>
    <tr>
        <th rowspan=2>Anonymous</th>
        <td>Create - N/A</td>
        <td bgcolor="orange">Read - Sometimes</td>
        <td bgcolor="red">Create - N/A</td>
        <td bgcolor="orange">Read - Sometimes</td>
        <td>Create - N/A</td>
        <td bgcolor="orange">Read - Sometimes</td>
    </tr>
    <tr>
        <td bgcolor="red">Update - Never</td>
        <td>Delete - N/A</td>
        <td bgcolor="red">Update - Never</td>
        <td>Delete - N/A</td>
        <td bgcolor="red">Update - Never</td>
        <td>Delete - N/A</td>
    </tr>
</table>

- AML nodes and Anonymous nodes may read clock, port state and some configuration values when the port they connect to is in the master state
- AML nodes may only update port state and clock when chosen as the best master by the BMCA
- System user may only read port state and configuration when allowed by system admin
- System admin may update port state, however this may result in unintended behaviour.

## Failure cases
<table>
    <tr>
        <th></th>
        <th colspan=2>Escalation of privilege</th>
        <th colspan=2>Denial of service</th>
    </tr>
    <tr>
        <th rowspan=2>Clock</th>
        <td>Create - N/A</td>
        <td bgcolor="green">Read - Low</td>
        <td>Create - N/A</td>
        <td bgcolor="orange">Read - Medium</td>
    </tr>
    <tr>
        <td bgcolor="red">Update - Critical</td>
        <td>Delete - N/A</td>
        <td bgcolor="orange">Update - Medium</td>
        <td>Delete - N/A</td>
    </tr>
    <tr>
        <th rowspan=2>Configuration</th>
        <td bgcolor="red">Create - Critical</td>
        <td bgcolor="green">Read - Low</td>
        <td bgcolor="green">Create - Low</td>
        <td bgcolor="green">Read - Low</td>
    </tr>
    <tr>
        <td bgcolor="red">Update - Critical</td>
        <td bgcolor="orange">Delete - Medium</td>
        <td bgcolor="green">Update - Low</td>
        <td bgcolor="green">Delete - Low</td>
    </tr>
    <tr>
        <th rowspan=2>Port State</th>
        <td>Create - N/A</td>
        <td bgcolor="green">Read - Low</td>
        <td>Create - N/A</td>
        <td bgcolor="orange">Read - Medium</td>
    </tr>
    <tr>
        <td bgcolor="red">Update - Critical</td>
        <td>Delete - N/A</td>
        <td bgcolor="orange">Update - Medium</td>
        <td>Delete - N/A</td>
    </tr>
</table>

## Security strategy

- Nodes with their clock identity not on the AML are not taken into account for the BMCA
- Time transmission messages are only accepted from the currently selected master
- Configuration files should not be world-writable
- A port marked master-only will never enter the slave state
