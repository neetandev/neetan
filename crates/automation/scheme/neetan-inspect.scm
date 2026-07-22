;; The public (neetan inspect 1) library.
(define-library (neetan inspect 1)
  (export
    processors processor-info registers register-ref
    protected-mode-state
    address-spaces address-space-info
    memory-read-bytevector memory-peek-unsigned
    save-memory!
    text-surfaces text-surface-info text-cell text-screen
    wait-for-text save-text-screen!)
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
          space address width byte-order)))

    (define (%require-string who value)
      (if (string? value)
          value
          (error (string-append who ": expected a string") 'neetan/argument)))

    (define default-maximum-ticks-per-frame 50000000)
    (define default-wait-for-text-frames 120)

    (define (save-memory! machine space address length path)
      (%require-symbol "save-memory!" space)
      (%require-count "save-memory!" address)
      (%require-count "save-memory!" length)
      (%require-string "save-memory!" path)
      (%raise-if-error
        (%save-memory (%require-machine-token "save-memory!" machine)
                      space address length path)))

    (define (text-surfaces machine)
      (%raise-if-error
        (%text-surfaces (%require-machine-token "text-surfaces" machine))))

    (define (text-surface-info machine surface)
      (%require-symbol "text-surface-info" surface)
      (%raise-if-error
        (%text-surface-info
          (%require-machine-token "text-surface-info" machine) surface)))

    (define (text-cell machine surface row column)
      (%require-symbol "text-cell" surface)
      (%require-count "text-cell" row)
      (%require-count "text-cell" column)
      (%raise-if-error
        (%text-cell (%require-machine-token "text-cell" machine)
                    surface row column)))

    (define (text-screen machine surface)
      (%require-symbol "text-screen" surface)
      (%raise-if-error
        (%text-screen (%require-machine-token "text-screen" machine) surface)))

    (define (save-text-screen! machine surface path)
      (%require-symbol "save-text-screen!" surface)
      (%require-string "save-text-screen!" path)
      (%raise-if-error
        (%save-text-screen
          (%require-machine-token "save-text-screen!" machine) surface path)))

    ;; Drives the machine until surface text matches the predicate, returning the
    ;; matched text or #f when the frame or tick bound is exhausted first.
    ;;
    ;; The predicate is a bare string, shorthand for ((contains . string)), or an
    ;; alist with a required 'contains string and an optional 'row index; options
    ;; is an optional alist with 'frames and 'ticks bounds.
    ;;
    ;; The text surface is the live text plane, sampled once before running and
    ;; then at each frame boundary. Text written and overwritten within a single
    ;; frame is not observed.
    (define (wait-for-text machine surface raw-predicate . optional-options)
      (%require-symbol "wait-for-text" surface)
      (define predicate
        (if (string? raw-predicate)
            (list (cons 'contains raw-predicate))
            raw-predicate))
      (if (not (list? predicate))
          (error "wait-for-text: predicate must be a string or an association list"
                 'neetan/argument))
      (let* ((missing (cons #f #f))
             (contains (alist-ref predicate 'contains missing))
             (row (alist-ref predicate 'row #f))
             (options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options)) (car optional-options))
                 (else
                   (error "wait-for-text: expected three or four arguments"
                          'neetan/argument))))
             (frames-value (alist-ref options 'frames missing))
             (ticks-value (alist-ref options 'ticks missing))
             (maximum-frames
               (if (eq? frames-value missing)
                   default-wait-for-text-frames
                   (%require-count "wait-for-text" frames-value)))
             (maximum-ticks
               (if (eq? ticks-value missing)
                   (* maximum-frames default-maximum-ticks-per-frame)
                   (%require-count "wait-for-text" ticks-value))))
        (if (eq? contains missing)
            (error "wait-for-text: predicate requires a 'contains string"
                   'neetan/argument))
        (%require-string "wait-for-text" contains)
        (if (and row (not (and (integer? row) (exact? row) (>= row 0))))
            (error "wait-for-text: row must be a non-negative exact integer or #f"
                   'neetan/argument))
        (%raise-if-error
          (%wait-for-text (%require-machine-token "wait-for-text" machine)
                          surface contains row maximum-frames maximum-ticks))))))
