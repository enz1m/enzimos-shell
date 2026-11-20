import json
import select
import sys
import time


def counters():
    with open('/proc/stat', encoding='ascii') as stat:
        fields = stat.readline().split()
    if not fields or fields[0] != 'cpu' or len(fields) < 8:
        raise ValueError('CPU counters are unavailable')
    values = [int(value) for value in fields[1:8]]
    return sum(values), values[3] + values[4]


class CpuMeter:
    def __init__(self, start):
        self.interval = start['config']['interval_seconds']
        self.previous = None
        self.history = []
        self.revision = 0
        self.status = 'Sampling CPU…'
        self.next_sample = time.monotonic()
        self.snapshot()

    def snapshot(self):
        self.revision += 1
        recent = ' · '.join('%d%%' % value for value in self.history[-5:])
        nodes = [
            {'type': 'text', 'key': 'current', 'text': self.status, 'tone': 'title'},
            {'type': 'text', 'key': 'history', 'text': 'Recent: ' + recent if recent else 'Waiting for samples', 'tone': 'normal'},
            {'type': 'text', 'key': 'source', 'text': 'Total host CPU · /proc/stat · read-only', 'tone': 'muted'},
        ]
        send({'type': 'snapshot', 'revision': self.revision, 'elements': [{
            'id': 'usage', 'icon': 'cpu', 'label': self.status[:48],
            'tooltip': 'Total CPU utilization, sampled every %s seconds' % self.interval,
            'panel': {'title': 'CPU meter', 'width': 320, 'height': 180, 'nodes': nodes},
        }]})

    def sample(self):
        try:
            current = counters()
            if self.previous is not None:
                total = current[0] - self.previous[0]
                idle = current[1] - self.previous[1]
                if total > 0 and 0 <= idle <= total:
                    used = min(100, max(0, round(100 * (total - idle) / total)))
                    self.status = '%d%% CPU' % used
                    self.history.append(used)
                    del self.history[:-24]
                else:
                    self.status = 'Sampling CPU…'
            self.previous = current
        except (OSError, ValueError):
            self.previous = None
            self.status = 'CPU counters unavailable'
        self.next_sample = time.monotonic() + self.interval
        self.snapshot()

    def message(self, message):
        if message['type'] == 'ping':
            send({'type': 'pong', 'sequence': message['sequence']})
        elif message['type'] == 'config':
            self.interval = message['config']['interval_seconds']
            self.next_sample = time.monotonic()
        elif message['type'] in ('action', 'provider', 'response'):
            return


def send(value):
    print(json.dumps(value, separators=(',', ':')), flush=True)


def main():
    send({'type': 'hello', 'protocol': 1, 'plugin': 'cpu-meter'})
    meter = CpuMeter(json.loads(sys.stdin.readline()))
    while True:
        timeout = max(0, meter.next_sample - time.monotonic())
        ready, _, _ = select.select([sys.stdin], [], [], min(timeout, 1.0))
        if ready:
            line = sys.stdin.readline()
            if not line:
                break
            meter.message(json.loads(line))
        if time.monotonic() >= meter.next_sample:
            meter.sample()


if __name__ == '__main__':
    main()
