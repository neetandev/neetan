//! INT 24h: Critical Error Handler (default).
//!
//! Our HLE DOS never raises critical errors internally: INT 21h file and
//! disk handlers return DOS error codes directly from host I/O. However,
//! guest software still reads IVT[24h], chains through the saved PSP+12h
//! vector, or issues INT 24h explicitly, so the vector must point at a
//! well-behaved handler.
//!
//! `boot()` installs a 3-byte in-RAM stub at `tables::INT24_STUB_ADDR`
//! in the DOS data segment (`MOV AL,3` / `IRET`) and points IVT[24h]
//! at it. AL=3 is "Fail" on DOS 3.3+. Because the stub executes
//! natively on the guest CPU, no Rust dispatch handler lives here.
