;; PC-98 inspection and mutation through the public libraries.
(import (scheme base)
        (neetan automation 1)
        (neetan inspect 1)
        (neetan mutate 1)
        (neetan test 1))

;; Returns #t when thunk raises an error carrying the given neetan symbol.
(define (raises? symbol thunk)
  (guard (condition
          (#t (and (error-object? condition)
                   (memq symbol (error-object-irritants condition))
                   #t)))
    (thunk)
    #f))

(test-suite "PC-98 inspection and mutation"
  (with-machine (machine '((target . pc98) (model . pc9801vm)))

(test-case "capabilities report inspect and mutate"
  (check-true (machine-capability? machine 'inspect))
  (check-true (machine-capability? machine 'mutate)))

(test-case "processor and space discovery"
  (check-equal '(cpu.main) (processors machine))
  (check-equal '(cpu.main.memory cpu.main.io) (address-spaces machine))
  (let ((info (processor-info machine 'cpu.main)))
    (check-equal 'x86 (cdr (assq 'architecture info)))
    (check-false (cdr (assq 'protected-mode info))))
  (let ((space (address-space-info machine 'cpu.main.memory)))
    (check-equal 20 (cdr (assq 'address-bits space)))
    (check-equal 'little (cdr (assq 'byte-order space)))
    (check-true (cdr (assq 'peekable space)))
    (check-true (cdr (assq 'writable space)))))

(test-case "register set and reference round-trip"
  (register-set! machine 'cpu.main 'ax #x1234)
  (check-equal #x1234 (register-ref machine 'cpu.main 'ax))
  (check-equal #x1234 (cdr (assq 'ax (registers machine 'cpu.main)))))

(test-case "memory poke and peek round-trip and byte order"
  (memory-write-bytevector! machine 'cpu.main.memory #x400 #u8(#x11 #x22 #x33 #x44))
  (check-equal #u8(#x11 #x22 #x33 #x44)
               (memory-read-bytevector machine 'cpu.main.memory #x400 4))
  (check-equal #x44332211
               (memory-peek-unsigned machine 'cpu.main.memory #x400 4 'little))
  (check-equal #x11223344
               (memory-peek-unsigned machine 'cpu.main.memory #x400 4 'big))
  (check-equal #x44332211
               (memory-peek-unsigned machine 'cpu.main.memory #x400 4 'native))
  (memory-poke-unsigned! machine 'cpu.main.memory #x400 2 'big #xABCD)
  (check-equal #u8(#xAB #xCD)
               (memory-read-bytevector machine 'cpu.main.memory #x400 2)))

(test-case "error contract"
  ;; The I/O space is descriptor-only.
  (check-true (raises? 'neetan/unsupported
                       (lambda () (memory-read-bytevector machine 'cpu.main.io 0 1))))
    ;; A V30 has no protected-mode state.
    (check-true (raises? 'neetan/unsupported
                         (lambda () (protected-mode-state machine 'cpu.main))))
    ;; The V30 memory space is 20 bits wide.
    (check-true (raises? 'neetan/range
                         (lambda () (memory-read-bytevector machine 'cpu.main.memory #x100000 1))))
    ;; A 16-bit register rejects an out-of-range value.
    (check-true (raises? 'neetan/range
                         (lambda () (register-set! machine 'cpu.main 'ax #x10000))))
    ;; Unknown processors, registers, and spaces are argument errors.
    (check-true (raises? 'neetan/argument
                         (lambda () (register-ref machine 'cpu.other 'ax))))
    (check-true (raises? 'neetan/argument
                         (lambda () (register-ref machine 'cpu.main 'zz))))
  (check-true (raises? 'neetan/argument
                       (lambda () (memory-read-bytevector machine 'cpu.main.vram 0 1)))))))
