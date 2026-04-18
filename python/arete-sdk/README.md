# arete-sdk

> **Work in Progress:** This SDK is under active development and has not yet been published to PyPI.

Python SDK for Arete - Real-time Solana program data streaming.

## Installation

```bash
# Not yet published - install from source for development
pip install -e .
```

## Requirements

- Python 3.9+

## Usage

```python
from arete import AreteClient

# Initialize client
client = AreteClient()

# Connect and subscribe to program data
await client.connect()
```

## Features

- Real-time WebSocket streaming
- Solana program state subscriptions
- Async/await support

## License

MIT

## Links

- [Repository](https://github.com/AdrianMAnderson/arete-oss)
- [Issues](https://github.com/AdrianMAnderson/arete-oss/issues)
