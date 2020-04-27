release: linux_release windows_release

linux_release:
	cargo build --release
	cd ./target/release/ && tar -cvzf ./cg-local-rs.tar.gz ./cg-local-rs

windows_release:
	cargo build --release --target x86_64-pc-windows-gnu 
	cd ./target/x86_64-pc-windows-gnu/release && zip ./cg-local-rs.zip ./cg-local-rs.exe
