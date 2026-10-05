"""In-memory transport and clock for supervisor tests."""

from argos.transport import Sample


class FakeTransport:
    def __init__(self):
        self.sent = []
        self.callbacks = []
        self.closed = False

    def put(self, key, payload):
        self.sent.append((key, bytes(payload)))

    def subscribe(self, key, callback):
        self.callbacks.append((key, callback))
        return key

    def inject(self, key, payload):
        sample = Sample(key, payload if isinstance(payload, bytes) else payload.encode())
        for subscribed, callback in self.callbacks:
            if subscribed == key:
                callback(sample)

    def close(self):
        self.closed = True


class Clock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, delay):
        self.now += delay
