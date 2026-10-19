import os
import sys

def is_emoji(char):
    code = ord(char)
    return (
        0x1F600 <= code <= 0x1F64F or
        0x1F300 <= code <= 0x1F5FF or
        0x1F680 <= code <= 0x1F6FF or
        0x1F700 <= code <= 0x1F77F or
        0x1F780 <= code <= 0x1F7FF or
        0x1F800 <= code <= 0x1F8FF or
        0x1F900 <= code <= 0x1F9FF or
        0x1FA00 <= code <= 0x1FA6F or
        0x1FA70 <= code <= 0x1FAFF or
        0x2600 <= code <= 0x26FF or
        0x2700 <= code <= 0x27BF or
        0x2300 <= code <= 0x23FF or
        code in (0xFE0F, 0x200D)
    )

def clean_rust_code(content):
    result = []
    i = 0
    n = len(content)

    while i < n:
        if content[i] == '"':
            result.append('"')
            i += 1
            while i < n:
                ch = content[i]
                if is_emoji(ch):
                    i += 1
                    continue
                result.append(ch)
                if ch == '\\' and i + 1 < n:
                    i += 1
                    result.append(content[i])
                elif ch == '"':
                    i += 1
                    break
                i += 1
            continue

        if content[i] == 'r' and i + 1 < n and content[i+1] in ('"', '#'):
            result.append(content[i])
            i += 1
            hashes = 0
            while i < n and content[i] == '#':
                hashes += 1
                result.append('#')
                i += 1
            if i < n and content[i] == '"':
                result.append('"')
                i += 1
                end_delim = '"' + '#' * hashes
                while i < n:
                    if content[i:i+len(end_delim)] == end_delim:
                        result.append(end_delim)
                        i += len(end_delim)
                        break
                    if not is_emoji(content[i]):
                        result.append(content[i])
                    i += 1
            continue

        if content[i] == "'":
            is_char = False
            if i + 2 < n and content[i+2] == "'":
                is_char = True
            elif i + 3 < n and content[i+1] == '\\' and content[i+3] == "'":
                is_char = True

            if is_char:
                result.append(content[i])
                i += 1
                while i < n:
                    ch = content[i]
                    if not is_emoji(ch):
                        result.append(ch)
                    if ch == '\\' and i + 1 < n:
                        i += 1
                        result.append(content[i])
                    elif ch == "'":
                        i += 1
                        break
                    i += 1
                continue

        if content[i:i+2] == '//':
            line_rest = content[i:]
            if line_rest.startswith('// SAFETY:') or line_rest.startswith('//SAFETY:'):
                while i < n and content[i] != '\n':
                    if not is_emoji(content[i]):
                        result.append(content[i])
                    i += 1
            else:
                while i < n and content[i] != '\n':
                    i += 1
            continue

        if content[i:i+2] == '/*':
            depth = 1
            i += 2
            while i < n and depth > 0:
                if content[i:i+2] == '/*':
                    depth += 1
                    i += 2
                elif content[i:i+2] == '*/':
                    depth -= 1
                    i += 2
                else:
                    i += 1
            continue

        ch = content[i]
        if not is_emoji(ch):
            result.append(ch)
        i += 1

    cleaned = "".join(result)
    lines = [line.rstrip() for line in cleaned.split('\n')]
    
    final_lines = []
    prev_empty = False
    for line in lines:
        if not line:
            if not prev_empty:
                final_lines.append("")
                prev_empty = True
        else:
            final_lines.append(line)
            prev_empty = False

    return "\n".join(final_lines) + "\n"

root_dir = "/home/sif/Project Life/Apex-Wire"
cleaned_count = 0

for dirpath, _, filenames in os.walk(root_dir):
    if "/target" in dirpath or "/.git" in dirpath:
        continue
    for filename in filenames:
        if filename.endswith(".rs"):
            filepath = os.path.join(dirpath, filename)
            with open(filepath, "r", encoding="utf-8", errors="ignore") as f:
                content = f.read()
            cleaned = clean_rust_code(content)
            with open(filepath, "w", encoding="utf-8") as f:
                f.write(cleaned)
            cleaned_count += 1

print(f"Processed {cleaned_count} Rust files successfully.")
