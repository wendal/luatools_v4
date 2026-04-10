// luadb/mod.rs
// Placeholder for the LuaDB filesystem packer.
#![allow(dead_code)]
//
// LuaDB is the LuatOS file-system image format used to bundle Lua scripts and
// resource files into a binary blob that is flashed to a dedicated partition
// on the chip.
//
// Future work:
//   - Parse an input directory tree.
//   - Compile `.lua` source files to `.luac` bytecode (Lua 5.3).
//   - Assemble the luadb header + file entries + file data.
//   - Write the resulting binary to a file or return it as `Vec<u8>`.
//
// Reference: yuzhan-tech/luatos-tools src/luadb/

/// Pack a directory of Lua scripts and assets into a luadb image.
///
/// `input_dir` — path to the directory containing the scripts to pack.
/// `output_path` — where to write the resulting `.bin` image.
///
/// Returns an error as long as the implementation is a stub.
pub fn pack_directory(_input_dir: &std::path::Path, _output_path: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("luadb packer is not yet implemented")
}
