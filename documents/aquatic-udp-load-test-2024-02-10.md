2024-02-10 Joakim Frostegård

# UDP BitTorrent tracker throughput comparison

This is a performance comparison of several UDP BitTorrent tracker implementations.

Benchmarks were run using [aquatic_bencher](../crates/bencher), with `--cpu-mode subsequent-one-per-pair`.

## Software and hardware

### Tracker implementations

| Name          | Commit  |
|---------------|---------|
| [aquatic_udp] | 21a5301 |
| [opentracker] | 110868e |
| [chihaya]     | 2f79440 |

[aquatic_udp]: ../crates/udp
[opentracker]: http://erdgeist.org/arts/software/opentracker/
[chihaya]: https://github.com/chihaya/chihaya

### OS and compilers

| Name   | Version |
|--------|---------|
| Debian | 12.4    |
| Linux  | 6.5.10  |
| rustc  | 1.76.0  |
| GCC    | 12.2.0  |
| go     | 1.19.8  |

### Hardware

Hetzner CCX63: 48 dedicated vCPUs (AMD Milan Epyc 7003)

## Results

![UDP BitTorrent tracker throughput](./aquatic-udp-load-test-2024-02-10.png)

<table>
    <caption>
        <strong>UDP BitTorrent tracker troughput</strong>
        <p>Average responses per second, best result.</p>
    </caption>
    <thead>
        <tr>
            <th>CPU cores</th>
            <th>aquatic_udp (mio)</th>
            <th>aquatic_udp (io_uring)</th>
            <th>opentracker</th>
            <th>chihaya</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <th>1</th>
            <td><span title="socket workers: 1, avg cpu utilization: 95.3%">186,939</span></td>
            <td><span title="socket workers: 1, avg cpu utilization: 95.3%">226,065</span></td>
            <td><span title="workers: 1, avg cpu utilization: 95.3%">190,540</span></td>
            <td><span title="avg cpu utilization: 95.3%">55,989</span></td>
        </tr>
        <tr>
            <th>2</th>
            <td><span title="socket workers: 2, avg cpu utilization: 190%">371,478</span></td>
            <td><span title="socket workers: 2, avg cpu utilization: 190%">444,353</span></td>
            <td><span title="workers: 2, avg cpu utilization: 190%">379,623</span></td>
            <td><span title="avg cpu utilization: 186%">111,226</span></td>
        </tr>
        <tr>
            <th>4</th>
            <td><span title="socket workers: 4, avg cpu utilization: 381%">734,709</span></td>
            <td><span title="socket workers: 4, avg cpu utilization: 381%">876,642</span></td>
            <td><span title="workers: 4, avg cpu utilization: 381%">748,401</span></td>
            <td><span title="avg cpu utilization: 300%">136,983</span></td>
        </tr>
        <tr>
            <th>6</th>
            <td><span title="socket workers: 6, avg cpu utilization: 565%">1,034,804</span></td>
            <td><span title="socket workers: 6, avg cpu utilization: 572%">1,267,006</span></td>
            <td><span title="workers: 6, avg cpu utilization: 567%">901,600</span></td>
            <td><span title="avg cpu utilization: 414%">131,827</span></td>
        </tr>
        <tr>
            <th>8</th>
            <td><span title="socket workers: 8, avg cpu utilization: 731%">1,296,693</span></td>
            <td><span title="socket workers: 8, avg cpu utilization: 731%">1,521,113</span></td>
            <td><span title="workers: 8, avg cpu utilization: 756%">1,170,928</span></td>
            <td><span title="avg cpu utilization: 462%">131,779</span></td>
        </tr>
        <tr>
            <th>12</th>
            <td><span title="socket workers: 12, avg cpu utilization: 1064%">1,871,353</span></td>
            <td><span title="socket workers: 12, avg cpu utilization: 957%">1,837,223</span></td>
            <td><span title="workers: 12, avg cpu utilization: 1127%">1,675,059</span></td>
            <td><span title="avg cpu utilization: 509%">130,942</span></td>
        </tr>
        <tr>
            <th>16</th>
            <td><span title="socket workers: 16, avg cpu utilization: 1126%">2,037,713</span></td>
            <td><span title="socket workers: 16, avg cpu utilization: 1109%">2,258,321</span></td>
            <td><span title="workers: 16, avg cpu utilization: 1422%">1,645,828</span></td>
            <td><span title="avg cpu utilization: 487%">127,256</span></td>
        </tr>
    </tbody>
</table>
