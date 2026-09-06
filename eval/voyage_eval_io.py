"""Bounded local evidence reads. These checks are not an OS sandbox."""
from __future__ import annotations
import json
import math
import os
from pathlib import Path, PurePosixPath
import stat

MAX_FILE = 4 * 1024 * 1024
MAX_TOTAL = 16 * 1024 * 1024


def relative(name: str) -> tuple[str, ...]:
    if not isinstance(name, str) or not name or '\\' in name or '\0' in name:
        raise ValueError('invalid evidence path')
    parts = name.split('/')
    if PurePosixPath(name).is_absolute() or any(p in ('', '.', '..') for p in parts):
        raise ValueError('evidence path must stay below its root')
    return tuple(parts)


def strict_json(data: str | bytes):
    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate JSON key')
            result[key] = value
        return result
    def reject_number(_):
        raise ValueError('nonfinite JSON number')

    def finite_float(value):
        result = float(value)
        if not math.isfinite(result):
            reject_number(value)
        return result

    return json.loads(data, object_pairs_hook=object_pairs, parse_constant=reject_number,
                      parse_float=finite_float)


class EvidenceReader:
    def __init__(self, root: Path):
        if os.name != 'posix' or not hasattr(os, 'O_NOFOLLOW'):
            raise ValueError('safe evaluation evidence reads require POSIX no-follow descriptors')
        self.root = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        self.total = 0

    def __enter__(self):
        return self

    def __exit__(self, *_):
        os.close(self.root)

    def _parent(self, parts):
        fd = os.dup(self.root)
        try:
            for part in parts:
                next_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                os.close(fd)
                fd = next_fd
            return fd
        except BaseException:
            os.close(fd)
            raise

    def files(self, directory: str, suffix: str, limit: int = 32) -> list[str]:
        fd = self._parent(relative(directory))
        try:
            names = []
            with os.scandir(fd) as entries:
                for entry in entries:
                    if len(names) >= limit:
                        raise ValueError('too many evidence directory entries')
                    names.append(entry.name)
            return sorted(f'{directory}/{name}' for name in names if name.endswith(suffix))
        finally:
            os.close(fd)

    def read(self, name: str, limit: int = MAX_FILE) -> bytes:
        parts = relative(name)
        parent = self._parent(parts[:-1])
        try:
            fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        finally:
            os.close(parent)
        try:
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
                raise ValueError('evidence must be a bounded regular file')
            chunks, size = [], 0
            while True:
                data = os.read(fd, min(65536, limit + 1 - size))
                if not data:
                    break
                size += len(data)
                self.total += len(data)
                if size > limit or self.total > MAX_TOTAL:
                    raise ValueError('evidence byte limit exceeded')
                chunks.append(data)
            return b''.join(chunks)
        finally:
            os.close(fd)

    def json(self, name: str):
        return strict_json(self.read(name))
