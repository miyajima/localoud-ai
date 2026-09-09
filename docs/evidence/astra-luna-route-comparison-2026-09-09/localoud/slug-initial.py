def slugify(text):
    result = []
    pending_separator = False

    for char in text:
        if "a" <= char <= "z" or "0" <= char <= "9":
            if pending_separator and result:
                result.append("-")
            result.append(char)
            pending_separator = False
        elif "A" <= char <= "Z":
            if pending_separator and result:
                result.append("-")
            result.append(chr(ord(char) + (ord("a") - ord("A"))))
            pending_separator = False
        else:
            pending_separator = bool(result)

    return "".join(result)
