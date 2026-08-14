local cwd = vim.fn.getcwd()
local dap = require("dap")

dap.adapters.gdb = {
	type = "executable",
	command = "rust-gdb",
	args = { "--interpreter=dap" }, -- 使用 args 数组，带等号
}

dap.configurations.rust = {
	{
		name = "QEMU Debug",
		type = "gdb",
		request = "attach",
		target = "localhost:1234",
		program = cwd .. "/target/riscv64gc-unknown-none-elf/debug/sqware",
		cwd = cwd,
		stopAtBeginningOfMainSubprogram = false, -- 修正字段名

		-- initCommands 可以放其他初始化命令（如设置断点条件、加载符号等）
		-- 但不要再放 target remote
		initCommands = {
			-- 例如：set print pretty on
		},

		-- 路径映射（编译时路径 → 本地源码路径）
		sourceMap = {
			["/build/"] = cwd,
			["/rustc/"] = "", -- 留空表示使用本地 rust 源码（需 rust-src 组件）
		},
	},
}
