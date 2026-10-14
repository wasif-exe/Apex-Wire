import os
import re
def clean_file_content(content, ext):
    res = []
    i = 0
    n = len(content)
    while i < n:
        if content[i] == '"':
            res.append(content[i])
            i += 1
            while i < n and content[i] != '"':
                if content[i] == '\\' and i + 1 < n:
                    res.append(content[i])
                    res.append(content[i+1])
                    i += 2
                else:
                    res.append(content[i])
                    i += 1
            if i < n:
                res.append(content[i])
                i += 1
        elif ext == ".rs" and content[i] == '\'' and i + 2 < n and (content[i+2] == '\'' or (content[i+1] == '\\' and i + 3 < n and content[i+3] == '\'')):
            res.append(content[i])
            i += 1
            while i < n and content[i] != '\'':
                if content[i] == '\\' and i + 1 < n:
                    res.append(content[i])
                    res.append(content[i+1])
                    i += 2
                else:
                    res.append(content[i])
                    i += 1
            if i < n:
                res.append(content[i])
                i += 1
        elif content[i:i+2] == '
            line_end = content.find('\n', i)
            if line_end == -1:
                line_end = n
            line_text = content[i:line_end]
            if "SAFETY:" in line_text:
                res.append(line_text)
            i = line_end
        elif content[i:i+2] == '', i + 2)
            if block_end == -1:
                i = n
            else:
                block_text = content[i:block_end+2]
                if "SAFETY:" in block_text:
                    res.append(block_text)
                i = block_end + 2
        elif ext in [".sh", ".py"] and content[i] == '
            if i == 0 and content[i:i+2] == '
                line_end = content.find('\n', i)
                if line_end == -1:
                    line_end = n
                res.append(content[i:line_end])
                i = line_end
            else:
                line_end = content.find('\n', i)
                if line_end == -1:
                    line_end = n
                i = line_end
        else:
            res.append(content[i])
            i += 1
    cleaned = "".join(res)
    cleaned_lines = [line for line in cleaned.splitlines() if line.strip() != ""]
    return "\n".join(cleaned_lines) + "\n"
def process_tree():
    root_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    for dirpath, _, filenames in os.walk(root_dir):
        if "target" in dirpath or ".git" in dirpath:
            continue
        for fname in filenames:
            _, ext = os.path.splitext(fname)
            if ext in [".rs", ".c", ".h", ".sh", ".py"]:
                fpath = os.path.join(dirpath, fname)
                with open(fpath, "r", encoding="utf-8") as f:
                    content = f.read()
                cleaned = clean_file_content(content, ext)
                with open(fpath, "w", encoding="utf-8") as f:
                    f.write(cleaned)
if __name__ == "__main__":
    process_tree()
    print("[+] Comment cleanup complete.")
