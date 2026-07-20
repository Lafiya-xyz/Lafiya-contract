import re, sys, pathlib, json

ROOT = pathlib.Path(__file__).resolve().parents[2]  # repo root

def get_contract_impl_functions(crate_path: pathlib.Path):
    src_path = crate_path / "src" / "lib.rs"
    content = src_path.read_text(encoding="utf-8")
    # Find the impl block with #[contractimpl]
    impl_blocks = re.split(r"@?\[contractimpl\]", content)
    functions = []
    for block in impl_blocks[1:]:  # after each marker
        # find all public functions within the block until next impl or end
        matches = re.finditer(r"pub fn\s+(\w+)\s*\(([^)]*)\)\s*->?\s*[^ {]*", block)
        for m in matches:
            name = m.group(1)
            args = m.group(2).replace("\n", " ").strip()
            functions.append((name, args))
    return functions

def parse_readme_functions(section_name: str, readme_path: pathlib.Path):
    content = readme_path.read_text(encoding="utf-8")
    pattern = rf"### `{section_name}`\s+\| Function \| Description \|.*?(?=\n\n|$)"
    match = re.search(pattern, content, re.DOTALL)
    if not match:
        return []
    table = match.group(0)
    # extract function names inside backticks
    funcs = re.findall(r"`([^`]+)`", table)
    signatures = []
    for f in funcs:
        if '(' in f:
            name, args = f.split('(', 1)
            args = args.rstrip(')')
            signatures.append((name.strip(), args.strip()))
        else:
            signatures.append((f.strip(), ""))
    return signatures

def main():
    repo_root = ROOT
    readme = repo_root / "README.md"
    crates = [repo_root / "contracts" / "attester-registry", repo_root / "contracts" / "attestation-registry"]
    all_ok = True
    for crate in crates:
        name = crate.name.replace('-', '_')
        impl_funcs = get_contract_impl_functions(crate)
        readme_funcs = parse_readme_functions(name, readme)
        impl_set = set(impl_funcs)
        readme_set = set(readme_funcs)
        missing = impl_set - readme_set
        extra = readme_set - impl_set
        if missing:
            all_ok = False
            print(f"Missing in README for {name}:")
            for f in missing:
                print(f"  {f[0]}({f[1]})")
        if extra:
            all_ok = False
            print(f"Extra in README for {name} (no impl):")
            for f in extra:
                print(f"  {f[0]}({f[1]})")
    if not all_ok:
        sys.exit(1)
    print("README contract function tables are in sync.")

if __name__ == "__main__":
    main()
