import re


_ASCII_LOWER = str.maketrans(
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "abcdefghijklmnopqrstuvwxyz",
)


def slugify(text):
    return re.sub(r"[^a-z0-9]+", "-", text.translate(_ASCII_LOWER)).strip("-")
