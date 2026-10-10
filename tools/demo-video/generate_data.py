"""Create a fictional drive index. Never read or create the indexed files."""
import json
import random
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "artifacts" / "diskvale-demo"
RNG = random.Random(20261009)
NOW = 1791504000
nodes = []
targets = {}


def add(name, parent=None, directory=False, gb=0, key=None):
    size = int(gb * 1_000_000_000)
    index = len(nodes)
    nodes.append(dict(name=name, parent=parent, children=[], directory=directory,
                      logical=size, allocated=((size + 4095) // 4096) * 4096,
                      files=int(not directory), modified=NOW - RNG.randrange(1, 750) * 86400,
                      shared=False, allocation_unknown=False))
    if parent is not None:
        nodes[parent]["children"].append(index)
    if key:
        targets[key] = index
    return index


def many(parent, prefix, extension, count, total_gb):
    weights = [RNG.uniform(.25, 1.75) for _ in range(count)]
    for i, weight in enumerate(weights):
        add(f"{prefix}_{i + 1:04d}.{extension}", parent, gb=total_gb * weight / sum(weights))


root = add("Z:\\", directory=True)
media = add("Media", root, True, key="media")
videos = add("Video", media, True, key="videos")
add("Showreel_4K.mp4", videos, gb=38.4, key="showreel")
add("Mountain_Film.mp4", videos, gb=21.6)
add("Studio_Reel.mp4", videos, gb=14.2)
add("Ocean_Timelapse.mp4", videos, gb=9.8)
add("Launch_Trailer.mp4", videos, gb=6.1)
many(videos, "Scene", "mp4", 36, 23.8)
photos = add("Photography", media, True)
many(photos, "Landscape", "raw", 240, 22.5)
many(photos, "Gallery", "jpg", 180, 5.3)
audio = add("Audio", media, True)
many(audio, "Studio_Session", "wav", 58, 8.6)

projects = add("Projects", root, True)
for title, total in [("Atlas", 45), ("Orbit", 29), ("Meadow", 18)]:
    project = add(title, projects, True)
    assets = add("Assets", project, True)
    many(assets, "Texture", "asset", 170, total * .53)
    many(assets, "Mesh", "fbx", 65, total * .3)
    cache = add("Build_Cache", project, True)
    many(cache, "Cached_Build", "bin", 100, total * .165)
    source = add("Source", project, True)
    many(source, "Module", "cs", 150, total * .005)

backups = add("Backups", root, True)
add("Studio_Backup_September.zip", backups, gb=54.2, key="backup")
add("Project_Archive_2025.7z", backups, gb=32.8)
add("Photo_Archive.zip", backups, gb=19.4)
many(backups, "Weekly_Backup", "zip", 8, 23.7)

games = add("Games", root, True)
for title, size in [("Starfall", 45.8), ("Ridge_Racer", 32.4), ("Garden_Quest", 17.6)]:
    game = add(title, games, True)
    content = add("Content", game, True)
    many(content, "Content", "pak", 110, size)

apps = add("Applications", root, True)
for title, size in [("Video_Studio", 11.5), ("Design_Tools", 8.7), ("Audio_Workbench", 4.4)]:
    app = add(title, apps, True)
    many(app, "Library", "dll", 75, size * .6)
    many(app, "Tool", "exe", 12, size * .4)

downloads = add("Downloads", root, True)
add("Sample_Footage_Pack.zip", downloads, gb=8.2)
add("Studio_Installer.iso", downloads, gb=5.6)
many(downloads, "Reference", "pdf", 50, 1.4)
many(downloads, "Sample_Clip", "mp4", 14, 4.8)
documents = add("Documents", root, True)
many(documents, "Project_Brief", "pdf", 70, 2.5)
many(documents, "Planning", "docx", 90, .6)
system = add("System", root, True)
add("hiberfil.sys", system, gb=12.8)
add("pagefile.sys", system, gb=8.6)
many(system, "System_Data", "dat", 125, 9.4)

for node in reversed(nodes):
    if node["directory"]:
        for field in ("logical", "allocated", "files"):
            node[field] = sum(nodes[i][field] for i in node["children"])
files = sum(not node["directory"] for node in nodes)
folders = len(nodes) - files
capacity = 1_000_000_000_000
stats = dict(files=files, folders=folders, skipped_links=0, errors=0,
             unknown_allocations=0, shared_links=0, elapsed_seconds=0, cancelled=False,
             engine="Synthetic demo index", notices=["DISKVALE_SYNTHETIC_DEMO_ONLY"],
             volume_space=dict(capacity=capacity, free=capacity - nodes[0]["allocated"],
                               free_at_start=capacity - nodes[0]["allocated"], captured_unix=NOW))
scan = dict(root="Z:\\", nodes=nodes, stats=stats)
serialized = json.dumps(scan, indent=2)
assert all(term not in serialized.lower() for term in ["c:\\users\\", "c:/users/", "\\appdata\\", "/appdata/"])
OUT.mkdir(parents=True, exist_ok=True)
(OUT / "demo-data.json").write_text(serialized, encoding="utf-8")
(OUT / "targets.json").write_text(json.dumps(targets, indent=2), encoding="utf-8")
print(f"Generated {files} fictional files, {folders} folders; {nodes[0]['allocated'] / 1e9:.2f} GB")
