;; The public (neetan inspect 1) library.
(define-library (neetan inspect 1)
  (export
    processors processor-info registers register-ref
    protected-mode-state
    address-spaces address-space-info
    memory-read-bytevector memory-peek-unsigned)
  (import (scheme base) (neetan automation 1) (neetan internal 1)
          (neetan handles internal 1))
  (begin
    (define (%raise-if-error value)
      (if (and (pair? value) (eq? (car value) '%error))
          (error (car (cddr value)) (cadr value))
          value))

    (define (%require-symbol who value)
      (if (symbol? value)
          value
          (error (string-append who ": expected a symbol") 'neetan/argument)))

    (define (%require-count who value)
      (if (and (integer? value) (exact? value) (>= value 0))
          value
          (error (string-append who ": expected a non-negative exact integer")
                 'neetan/argument)))

    (define (%require-positive who value)
      (if (and (integer? value) (exact? value) (> value 0))
          value
          (error (string-append who ": expected a positive exact integer")
                 'neetan/argument)))

    (define (%require-byte-order who value)
      (if (and (symbol? value) (memq value '(little big native)))
          value
          (error (string-append who ": byte order must be little, big, or native")
                 'neetan/argument)))

    (define (processors machine)
      (%raise-if-error
        (%processors (%require-machine-token "processors" machine))))

    (define (processor-info machine processor)
      (%require-symbol "processor-info" processor)
      (%raise-if-error
        (%processor-info
          (%require-machine-token "processor-info" machine) processor)))

    (define (registers machine processor)
      (%require-symbol "registers" processor)
      (%raise-if-error
        (%registers (%require-machine-token "registers" machine) processor)))

    (define (register-ref machine processor register)
      (%require-symbol "register-ref" processor)
      (%require-symbol "register-ref" register)
      (%raise-if-error
        (%register-ref
          (%require-machine-token "register-ref" machine) processor register)))

    (define (protected-mode-state machine processor)
      (%require-symbol "protected-mode-state" processor)
      (%raise-if-error
        (%protected-mode-state
          (%require-machine-token "protected-mode-state" machine) processor)))

    (define (address-spaces machine)
      (%raise-if-error
        (%address-spaces
          (%require-machine-token "address-spaces" machine))))

    (define (address-space-info machine space)
      (%require-symbol "address-space-info" space)
      (%raise-if-error
        (%address-space-info
          (%require-machine-token "address-space-info" machine) space)))

    (define (memory-read-bytevector machine space address length)
      (%require-symbol "memory-read-bytevector" space)
      (%require-count "memory-read-bytevector" address)
      (%require-count "memory-read-bytevector" length)
      (%raise-if-error
        (%memory-read-bytevector
          (%require-machine-token "memory-read-bytevector" machine)
          space address length)))

    (define (memory-peek-unsigned machine space address width byte-order)
      (%require-symbol "memory-peek-unsigned" space)
      (%require-count "memory-peek-unsigned" address)
      (%require-positive "memory-peek-unsigned" width)
      (%require-byte-order "memory-peek-unsigned" byte-order)
      (%raise-if-error
        (%memory-peek-unsigned
          (%require-machine-token "memory-peek-unsigned" machine)
          space address width byte-order)))))
