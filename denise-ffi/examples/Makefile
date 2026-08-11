# Builds the C example against the cdylib cargo produces.
#
#   cargo build -p denise-ffi --release
#   make && ./panel panel.ppm
#
# PROFILE=debug uses target/debug instead.

PROFILE ?= release
TARGET_DIR ?= ../../target/$(PROFILE)

CFLAGS += -std=c99 -Wall -Wextra -Wpedantic -I../include
LDFLAGS += -L$(TARGET_DIR)
LDLIBS += -ldenise_ffi

# So the binary finds the shared library without LD_LIBRARY_PATH, which is the
# difference between "make && ./panel" working and not.
UNAME := $(shell uname -s)
ifeq ($(UNAME),Darwin)
  LDFLAGS += -Wl,-rpath,$(abspath $(TARGET_DIR))
else
  LDFLAGS += -Wl,-rpath,$(abspath $(TARGET_DIR)) -Wl,--enable-new-dtags
endif

panel: panel.c ../include/denise.h
	$(CC) $(CFLAGS) -o $@ panel.c $(LDFLAGS) $(LDLIBS)

.PHONY: run clean
run: panel
	./panel panel.ppm

clean:
	rm -f panel panel.ppm
