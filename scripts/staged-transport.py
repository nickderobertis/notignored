#!/usr/bin/env python3
"""Serve staged assets at the installer's forge boundary on loopback only."""

import argparse
import functools
import http.server
import sys
from pathlib import Path


class Assets(http.server.SimpleHTTPRequestHandler):
    def do_GET(self) -> None:
        self.path = "/" + self.path.rsplit("/", 1)[-1]
        super().do_GET()

    def log_message(self, format: str, *args: object) -> None:
        pass


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assets", type=Path)
    parser.add_argument("port_file", type=Path)
    args = parser.parse_args()
    try:
        if not args.assets.is_dir():
            raise ValueError("staged asset directory is missing")
        if not args.port_file.parent.is_dir() or args.port_file.exists():
            raise ValueError("port destination must be a new file in an existing directory")
        with http.server.ThreadingHTTPServer(
            ("127.0.0.1", 0), functools.partial(Assets, directory=str(args.assets))
        ) as server:
            args.port_file.write_text(str(server.server_port))
            server.serve_forever()
    except (OSError, ValueError) as error:
        print(f"staged-transport: {error}\nACTION: supply an existing asset directory and a new writable port-file path", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
