release: linux_release windows_release

linux_release:
	cargo build --release

windows_release:
	cargo build --release --target x86_64-pc-windows-gnu 
