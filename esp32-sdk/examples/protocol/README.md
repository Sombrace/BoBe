# Portable protocol component

This example builds only the BodyLink codec and its JSON dependency. It encodes
example messages to the serial log; it does not open a network connection.

```sh
tools/idf.sh -C examples/protocol -B build-protocol -D IDF_TARGET=esp32c3 build
```

The same library can be built with native CMake without ESP-IDF, used from C or
C++, or added to an existing ESP-IDF application's component manifest.
