# Items as many to a line as fit in 120 columns, each line after `indent`.
def wrapped(indent, items):
    lines = [indent]
    for item in items:
        if len(lines[-1]) > len(indent) and len(lines[-1]) + 1 + len(item) > 120:
            lines.append(indent)
        lines[-1] += item if len(lines[-1]) == len(indent) else " " + item
    return lines
