;; The public (neetan mutate 1) library.
(define-library (neetan mutate 1)
  (export register-set! memory-write-bytevector! memory-poke-unsigned!)
  (import (scheme base) (neetan inspect 1) (neetan internal 1)
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

    (define (%require-bytevector who value)
      (if (bytevector? value)
          value
          (error (string-append who ": expected a bytevector") 'neetan/argument)))

    (define (register-set! machine processor register value)
      (%require-symbol "register-set!" processor)
      (%require-symbol "register-set!" register)
      (%require-count "register-set!" value)
      (%raise-if-error
        (%register-set (%require-machine-token "register-set!" machine)
                       processor register value))
      (if #f #f))

    (define (memory-write-bytevector! machine space address bytes)
      (%require-symbol "memory-write-bytevector!" space)
      (%require-count "memory-write-bytevector!" address)
      (%require-bytevector "memory-write-bytevector!" bytes)
      (%raise-if-error
        (%memory-write-bytevector
          (%require-machine-token "memory-write-bytevector!" machine)
          space address bytes))
      (if #f #f))

    (define (memory-poke-unsigned! machine space address width byte-order value)
      (%require-symbol "memory-poke-unsigned!" space)
      (%require-count "memory-poke-unsigned!" address)
      (%require-positive "memory-poke-unsigned!" width)
      (%require-byte-order "memory-poke-unsigned!" byte-order)
      (%require-count "memory-poke-unsigned!" value)
      (%raise-if-error
        (%memory-poke-unsigned
          (%require-machine-token "memory-poke-unsigned!" machine)
          space address width byte-order value))
      (if #f #f))))
