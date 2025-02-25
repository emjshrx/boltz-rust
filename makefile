.DEFAULT_GOAL := help
PROJECTNAME=$(shell basename "$(PWD)")
SOURCES=$(sort $(wildcard ./src/*.rs ./src/**/*.rs))

OS_NAME=$(shell uname | tr '[:upper:]' '[:lower:]')
PATH := $(ANDROID_NDK_HOME)/toolchains/llvm/prebuilt/$(OS_NAME)-x86_64/bin:$(PATH)

AR=llvm-ar

ANDROID_AARCH64_LINKER=$(ANDROID_NDK_HOME)/toolchains/llvm/prebuilt/$(OS_NAME)-x86_64/bin/aarch64-linux-android29-clang
ANDROID_ARMV7_LINKER=$(ANDROID_NDK_HOME)/toolchains/llvm/prebuilt/$(OS_NAME)-x86_64/bin/armv7a-linux-androideabi29-clang
ANDROID_I686_LINKER=$(ANDROID_NDK_HOME)/toolchains/llvm/prebuilt/$(OS_NAME)-x86_64/bin/i686-linux-android29-clang
ANDROID_X86_64_LINKER=$(ANDROID_NDK_HOME)/toolchains/llvm/prebuilt/$(OS_NAME)-x86_64/bin/x86_64-linux-android29-clang

SHELL := /bin/bash

BUILD_MODE ?= release

ifeq ($(BUILD_MODE),release)
	CARGO_BUILD_FLAGS = --release
else
	CARGO_BUILD_FLAGS =
endif

# ##############################################################################
# # GENERAL
# ##############################################################################

.PHONY: help
help: makefile
	@echo
	@echo " Available actions in "$(PROJECTNAME)":"
	@echo
	@sed -n 's/^##//p' $< | column -t -s ':' |  sed -e 's/^/ /'
	@echo

## init: Install missing dependencies.
.PHONY: init
init:
	rustup target add aarch64-apple-ios x86_64-apple-ios
	rustup target add aarch64-apple-darwin x86_64-apple-darwin
	#rustup target add armv7-apple-ios armv7s-apple-ios i386-apple-ios ## deprecated
	rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
	@if [ $$(uname) == "Darwin" ] ; then cargo install cargo-lipo ; fi
	cargo install cbindgen

## :

# ##############################################################################
# # RECIPES
# ##############################################################################

## all: Compile iOS, Android and bindings targets
##   Run `BUILD_MODE=debug make all` to build in debug mode
all: ios android bindings copy

## ios: Compile the iOS universal library
ios: target/universal/$(BUILD_MODE)/libboltz_rust.a

target/universal/$(BUILD_MODE)/libboltz_rust.a: $(SOURCES) ndk-home
	@if [ $$(uname) == "Darwin" ] ; then \
		cargo lipo $(CARGO_BUILD_FLAGS) ; \
		else echo "Skipping iOS compilation on $$(uname)" ; \
	fi
	@echo "[DONE] $@"

## macos: Compile the macOS libraries
macos: target/x86_64-apple-darwin/$(BUILD_MODE)/libboltz_rust.dylib target/aarch64-apple-darwin/$(BUILD_MODE)/libboltz_rust.dylib

target/x86_64-apple-darwin/$(BUILD_MODE)/libboltz_rust.dylib: $(SOURCES)
	@if [ $$(uname) == "Darwin" ] ; then \
		cargo lipo $(CARGO_BUILD_FLAGS) --targets x86_64-apple-darwin ; \
		else echo "Skipping macOS compilation on $$(uname)" ; \
	fi
	@echo "[DONE] $@"

target/aarch64-apple-darwin/$(BUILD_MODE)/libboltz_rust.dylib: $(SOURCES)
	@if [ $$(uname) == "Darwin" ] ; then \
		cargo lipo $(CARGO_BUILD_FLAGS) --targets aarch64-apple-darwin ; \
		else echo "Skipping macOS compilation on $$(uname)" ; \
	fi
	@echo "[DONE] $@"

## android: Compile the android targets (arm64, armv7 and i686)
android: target/aarch64-linux-android/$(BUILD_MODE)/libboltz_rust.so target/armv7-linux-androideabi/$(BUILD_MODE)/libboltz_rust.so target/i686-linux-android/$(BUILD_MODE)/libboltz_rust.so target/x86_64-linux-android/$(BUILD_MODE)/libboltz_rust.so

target/aarch64-linux-android/$(BUILD_MODE)/libboltz_rust.so: $(SOURCES) ndk-home
	CC_aarch64_linux_android=$(ANDROID_AARCH64_LINKER) \
	CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$(ANDROID_AARCH64_LINKER) \
		AR=llvm-ar cargo build --target aarch64-linux-android $(CARGO_BUILD_FLAGS)
	@echo "[DONE] $@"

target/armv7-linux-androideabi/$(BUILD_MODE)/libboltz_rust.so: $(SOURCES) ndk-home
	CC_armv7_linux_androideabi=$(ANDROID_ARMV7_LINKER) \
	CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER=$(ANDROID_ARMV7_LINKER) \
		AR=llvm-ar cargo build --target armv7-linux-androideabi $(CARGO_BUILD_FLAGS)
	@echo "[DONE] $@"

target/i686-linux-android/$(BUILD_MODE)/libboltz_rust.so: $(SOURCES) ndk-home
	CC_i686_linux_android=$(ANDROID_I686_LINKER) \
	CARGO_TARGET_I686_LINUX_ANDROID_LINKER=$(ANDROID_I686_LINKER) \
		AR=llvm-ar cargo build --target i686-linux-android $(CARGO_BUILD_FLAGS)
	@echo "[DONE] $@"

target/x86_64-linux-android/$(BUILD_MODE)/libboltz_rust.so: $(SOURCES) ndk-home
	CC_x86_64_linux_android=$(ANDROID_X86_64_LINKER) \
	CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER=$(ANDROID_X86_64_LINKER) \
		AR=llvm-ar cargo build --target x86_64-linux-android $(CARGO_BUILD_FLAGS)
	@echo "[DONE] $@"

.PHONY: ndk-home
ndk-home:
	@if [ ! -d "${ANDROID_NDK_HOME}" ] ; then \
		echo "Error: Please, set the ANDROID_NDK_HOME env variable to point to your NDK folder" ; \
		exit 1 ; \
	fi

## bindings: Generate the .h file for iOS
bindings: target/bindings.h

target/bindings.h: $(SOURCES)
	cbindgen --config cbindgen.toml --crate boltz_rust --output target/bindings.h
	@echo "[DONE] $@"

copy:
	rm -rf boltz-rust
	mkdir -p boltz-rust/android/app/src/main/jniLibs/arm64-v8a/ boltz-rust/android/app/src/main/jniLibs/armeabi-v7a/ boltz-rust/android/app/src/main/jniLibs/x86/ boltz-rust/android/app/src/main/jniLibs/x86_64/ boltz-rust/ios

	@echo target
	@echo $(BUILD_MODE)

	cp target/aarch64-linux-android/$(BUILD_MODE)/libboltz_rust.so boltz-rust/android/app/src/main/jniLibs/arm64-v8a/
	cp target/armv7-linux-androideabi/$(BUILD_MODE)/libboltz_rust.so boltz-rust/android/app/src/main/jniLibs/armeabi-v7a/
	cp target/i686-linux-android/$(BUILD_MODE)/libboltz_rust.so boltz-rust/android/app/src/main/jniLibs/x86/
	cp target/x86_64-linux-android/$(BUILD_MODE)/libboltz_rust.so boltz-rust/android/app/src/main/jniLibs/x86_64/
	cp target/bindings.h boltz-rust/
	cp target/universal/$(BUILD_MODE)/libboltz_rust.a boltz-rust/ios
	tar -cvzf boltz-rust.tar.gz boltz-rust

	mv boltz-rust.tar.gz boltz-rust-0.1.7.tar.gz

## :

# ##############################################################################
# # OTHER
# ##############################################################################

## clean:
.PHONY: clean
clean:
	cargo clean
	rm -f target/bindings.h target/bindings.src.h

## test:
.PHONY: test
test:
	cargo test
