;; The public (neetan automation 1) library.
(define-library (neetan automation 1)
  (export
    neetan-api-version
    host-config config-value alist-ref alist-key?
    call-with-machine with-machine machine? machine-state?
    machine-info machine-capabilities machine-capability?
    execution-result
    machine-epoch machine-tick machine-frame
    machine-epoch-tick machine-epoch-frame
    machine-emulated-time-ns machine-shutdown-requested?
    run-ticks! run-frames! run-until-frame! wait-until
    reset! restore-startup!
    save-state restore-state! discard-state! call-with-saved-state
    key-down! key-up! key-tap! type-text!
    joystick-set! joystick-clear! mouse-move! mouse-button!
    media-insert! media-eject! media-flush! media-info create-hdd! format-hdd!
    screen-available? screen-size screen-rgba screen-pixel screen-hash
    save-screenshot! screen-matches? screen-region-matches? wait-for-screen)
  (import (scheme base) (neetan internal 1) (neetan handles internal 1))
  (begin
    (define (%raise-if-error value)
      (if (and (pair? value) (eq? (car value) '%error))
          (error (car (cddr value)) (cadr value))
          value))

    (define (%require-symbol who value)
      (if (symbol? value)
          value
          (error (string-append who ": expected a symbol") 'neetan/argument)))

    (define (%require-string who value)
      (if (string? value)
          value
          (error (string-append who ": expected a string") 'neetan/argument)))

    (define (%require-integer who value)
      (if (and (integer? value) (exact? value))
          value
          (error (string-append who ": expected an exact integer") 'neetan/argument)))

    (define (%require-count who value)
      (if (and (integer? value) (exact? value) (>= value 0))
          value
          (error (string-append who ": expected a non-negative exact integer")
                 'neetan/argument)))

    (define (%require-boolean who value)
      (if (boolean? value)
          value
          (error (string-append who ": expected a boolean") 'neetan/argument)))

    (define (%validate-options who options allowed-keys)
      (if (not (list? options))
          (error (string-append who ": options must be an association list")
                 'neetan/argument)
          (let loop ((entries options) (keys '()))
            (if (null? entries)
                options
                (let ((entry (car entries)))
                  (if (or (not (pair? entry)) (not (symbol? (car entry))))
                      (error (string-append who ": malformed option")
                             'neetan/argument)
                      (if (not (memq (car entry) allowed-keys))
                          (error (string-append who ": unknown option")
                                 'neetan/argument)
                          (if (memq (car entry) keys)
                              (error (string-append who ": duplicate option")
                                     'neetan/argument)
                              (loop (cdr entries)
                                    (cons (car entry) keys))))))))))

    (define (%timeout-error? condition)
      (and (error-object? condition)
           (and (memq 'neetan/timeout (error-object-irritants condition)) #t)))

    (define (%stale-handle-error? condition)
      (and (error-object? condition)
           (and (memq 'neetan/stale-handle
                      (error-object-irritants condition))
                #t)))

    (define (neetan-api-version)
      (%api-version))

    (define machine? %machine-handle?)
    (define machine-state? %state-handle?)

    (define (%machine-token who machine)
      (%require-machine-token who machine))

    (define (execution-result tag . rest)
      (cond
        ((eq? tag 'OK)
         (if (%result-ok)
             (if #f #f)
             (error "execution result was already set" 'neetan/result-state)))
        ((eq? tag 'ERROR)
         (let ((message (if (pair? rest) (car rest) "")))
           (if (string? message)
               (if (%result-error message)
                   (if #f #f)
                   (error "execution result was already set" 'neetan/result-state))
               (error "execution-result message must be a string" 'neetan/argument))))
        (else
         (error "execution-result tag must be OK or ERROR" 'neetan/argument))))

    (define (host-config)
      (%host-config))

    (define (config-value key . rest)
      (%require-symbol "config-value" key)
      (if (or (not (list? rest)) (> (length rest) 1))
          (error "config-value: expected at most one default value"
                 'neetan/argument)
          (alist-ref (%host-config) key
                     (if (pair? rest) (car rest) #f))))

    (define (alist-ref alist key . rest)
      (%require-symbol "alist-ref" key)
      (if (or (not (list? rest)) (> (length rest) 1))
          (error "alist-ref: expected at most one default value" 'neetan/argument)
          (if (not (list? alist))
              (error "alist-ref: expected an association list" 'neetan/argument)
              (let loop ((entries alist) (found #f) (keys '()))
                (if (null? entries)
                    (if found
                        (cdr found)
                        (if (pair? rest)
                            (car rest)
                            (error "alist-ref: key is not present" 'neetan/argument)))
                    (let ((entry (car entries)))
                      (if (or (not (pair? entry)) (not (symbol? (car entry))))
                          (error "alist-ref: malformed association list" 'neetan/argument)
                          (if (memq (car entry) keys)
                              (error "alist-ref: duplicate key" 'neetan/argument)
                              (loop (cdr entries)
                                    (if (eq? (car entry) key) entry found)
                                    (cons (car entry) keys))))))))))

    (define (alist-key? alist key)
      (let ((marker (cons #f #f)))
        (not (eq? marker (alist-ref alist key marker)))))

    (define (call-with-machine specification procedure)
      (if (not (list? specification))
          (error "call-with-machine: specification must be an alist"
                 'neetan/argument)
          (begin
            ;; Validate the whole alist before crossing the native boundary.
            (alist-key? specification 'model)
            (if (not (procedure? procedure))
                (error "call-with-machine: expected a procedure" 'neetan/argument)
                (let ((token (%raise-if-error (%open-machine specification)))
                      (machine #f))
                  (dynamic-wind
                    (lambda () (if #f #f))
                    (lambda ()
                      (if (not machine)
                          (set! machine (%make-machine-handle token)))
                      (procedure machine))
                    (lambda ()
                      (guard (condition
                              ((%stale-handle-error? condition) (if #f #f)))
                        (%raise-if-error (%close-machine token))))))))))

    (define-syntax with-machine
      (syntax-rules ()
        ((_ (machine specification) body ...)
         (call-with-machine specification
           (lambda (machine)
             body ...)))))

    (define (machine-info machine)
      (let ((info (%machine-info (%machine-token "machine-info" machine))))
        (if info
            info
            (error "no machine has been constructed" 'neetan/no-machine))))

    (define (machine-capabilities machine)
      (let ((capabilities
              (%machine-capabilities
                (%machine-token "machine-capabilities" machine))))
        (if capabilities
            capabilities
            (error "no machine has been constructed" 'neetan/no-machine))))

    (define (machine-capability? machine capability)
      (%require-symbol "machine-capability?" capability)
      (%machine-capability?
        (%machine-token "machine-capability?" machine) capability))

    (define (machine-epoch machine)
      (%current-epoch (%machine-token "machine-epoch" machine)))
    (define (machine-tick machine)
      (%current-tick (%machine-token "machine-tick" machine)))
    (define (machine-frame machine)
      (%current-frame (%machine-token "machine-frame" machine)))
    (define (machine-epoch-tick machine)
      (%epoch-tick (%machine-token "machine-epoch-tick" machine)))
    (define (machine-epoch-frame machine)
      (%epoch-frame (%machine-token "machine-epoch-frame" machine)))
    (define (machine-emulated-time-ns machine)
      (%emulated-time-ns
        (%machine-token "machine-emulated-time-ns" machine)))
    (define (machine-shutdown-requested? machine)
      (%shutdown-requested?
        (%machine-token "machine-shutdown-requested?" machine)))

    (define (run-ticks! machine count)
      (%require-count "run-ticks!" count)
      (%raise-if-error
        (%run-ticks (%machine-token "run-ticks!" machine) count)))

    (define default-maximum-ticks-per-frame 50000000)

    (define (run-frames! machine count . optional-maximum-ticks)
      (%require-count "run-frames!" count)
      (let ((maximum-ticks
              (cond
                ((null? optional-maximum-ticks)
                 (* count default-maximum-ticks-per-frame))
                ((null? (cdr optional-maximum-ticks))
                 (car optional-maximum-ticks))
                (else
                  (error "run-frames!: expected two or three arguments"
                         'neetan/argument)))))
        (%require-count "run-frames!" maximum-ticks)
        (%raise-if-error
          (%run-frames
            (%machine-token "run-frames!" machine) count maximum-ticks))))

    (define (run-until-frame! machine frame . optional-options)
      (%require-count "run-until-frame!" frame)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "run-until-frame!" (car optional-options) '(ticks)))
                 (else
                   (error "run-until-frame!: expected two or three arguments"
                          'neetan/argument))))
             (machine-token (%machine-token "run-until-frame!" machine))
             (remaining-frames (max (- frame (%current-frame machine-token)) 0))
             (missing (cons #f #f))
             (ticks-value (alist-ref options 'ticks missing))
             (maximum-ticks
               (if (eq? ticks-value missing)
                   (* remaining-frames default-maximum-ticks-per-frame)
                   (%require-count "run-until-frame!" ticks-value))))
        (%raise-if-error
          (%run-until-frame machine-token frame maximum-ticks))))

    (define default-wait-until-frames 1800)

    (define (wait-until machine predicate . optional-options)
      (%machine-token "wait-until" machine)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "wait-until" (car optional-options) '(frames ticks)))
                 (else
                   (error "wait-until: expected two or three arguments"
                          'neetan/argument))))
             (missing (cons #f #f))
             (frames-value (alist-ref options 'frames missing))
             (ticks-value (alist-ref options 'ticks missing))
             (maximum-frames
               (if (eq? frames-value missing)
                   default-wait-until-frames
                   (%require-count "wait-until" frames-value)))
             (maximum-ticks
               (if (eq? ticks-value missing)
                   (* maximum-frames default-maximum-ticks-per-frame)
                   (%require-count "wait-until" ticks-value))))
        (let loop ((frames-left maximum-frames) (ticks-left maximum-ticks))
          (cond
            ((predicate) #t)
            ((or (<= frames-left 0) (<= ticks-left 0)) #f)
            (else
             (let ((used
                     (guard (condition ((%timeout-error? condition) #f))
                       (alist-ref
                         (run-frames! machine 1 ticks-left) 'ticks))))
               (if used
                   (loop (- frames-left 1) (- ticks-left used))
                   #f)))))))

    (define (reset! machine kind)
      (%require-symbol "reset!" kind)
      (%raise-if-error (%reset (%machine-token "reset!" machine) kind))
      (if #f #f))

    (define (restore-startup! machine)
      (%raise-if-error
        (%restore-startup (%machine-token "restore-startup!" machine)))
      (if #f #f))

    (define (save-state machine)
      (%make-state-handle
        machine
        (%raise-if-error (%save-state (%machine-token "save-state" machine)))))

    (define (restore-state! machine state)
      (%machine-token "restore-state!" machine)
      (if (not (%state-handle? state))
          (error "restore-state!: expected a machine state" 'neetan/argument)
          (if (not (eq? machine (%state-owner state)))
              (error "restore-state!: state belongs to another machine"
                     'neetan/stale-handle)
              (%raise-if-error
                (%restore-state
                  (%machine-token "restore-state!" machine)
                  (%state-token state)))))
      (if #f #f))

    (define (discard-state! state)
      (if (not (%state-handle? state))
          (error "discard-state!: expected a machine state" 'neetan/argument)
          (begin
            (%machine-token "discard-state!" (%state-owner state))
            (%raise-if-error
              (%discard-state
                (%machine-token "discard-state!" (%state-owner state))
                (%state-token state)))))
      (if #f #f))

    (define (call-with-saved-state machine procedure)
      (let ((machine-token
              (%machine-token "call-with-saved-state" machine)))
        (if (not (procedure? procedure))
            (error "call-with-saved-state: expected a procedure" 'neetan/argument)
            (let ((state-token (%raise-if-error (%save-state machine-token)))
                  (state #f))
              (dynamic-wind
                (lambda () (if #f #f))
                (lambda ()
                  (if (not state)
                      (set! state (%make-state-handle machine state-token)))
                  (procedure state))
                (lambda ()
                  (guard (condition
                          ((%stale-handle-error? condition) (if #f #f)))
                    (%raise-if-error
                      (%discard-state machine-token state-token)))))))))

    (define (key-down! machine key)
      (%require-symbol "key-down!" key)
      (%raise-if-error (%key-down (%machine-token "key-down!" machine) key))
      (if #f #f))

    (define (key-up! machine key)
      (%require-symbol "key-up!" key)
      (%raise-if-error (%key-up (%machine-token "key-up!" machine) key))
      (if #f #f))

    (define default-key-tap-frames 2)

    (define (key-tap! machine key . optional-options)
      (%require-symbol "key-tap!" key)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "key-tap!" (car optional-options) '(frames ticks)))
                 (else
                   (error "key-tap!: expected two or three arguments"
                          'neetan/argument))))
             (missing (cons #f #f))
             (frames-value (alist-ref options 'frames missing))
             (ticks-value (alist-ref options 'ticks missing))
             (frames
               (if (eq? frames-value missing)
                   default-key-tap-frames
                   (%require-count "key-tap!" frames-value)))
             (maximum-ticks
               (if (eq? ticks-value missing)
                   (* frames default-maximum-ticks-per-frame)
                   (%require-count "key-tap!" ticks-value))))
        (%raise-if-error
          (%key-tap (%machine-token "key-tap!" machine)
                    key frames maximum-ticks))))

    (define default-type-text-frames 2)

    (define (type-text! machine text . optional-options)
      (%require-string "type-text!" text)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "type-text!" (car optional-options) '(frames ticks)))
                 (else
                   (error "type-text!: expected two or three arguments"
                          'neetan/argument))))
             (missing (cons #f #f))
             (frames-value (alist-ref options 'frames missing))
             (ticks-value (alist-ref options 'ticks missing))
             (spacing-frames
               (if (eq? frames-value missing)
                   default-type-text-frames
                   (%require-count "type-text!" frames-value)))
             (maximum-ticks-per-character
               (if (eq? ticks-value missing)
                   (* spacing-frames default-maximum-ticks-per-frame)
                   (%require-count "type-text!" ticks-value))))
        (%raise-if-error
          (%type-text (%machine-token "type-text!" machine)
                      text spacing-frames maximum-ticks-per-character))
        (if #f #f)))

    (define (joystick-set! machine index control pressed?)
      (%require-count "joystick-set!" index)
      (%require-symbol "joystick-set!" control)
      (%require-boolean "joystick-set!" pressed?)
      (%raise-if-error
        (%joystick-set (%machine-token "joystick-set!" machine)
                       index control pressed?))
      (if #f #f))

    (define (joystick-clear! machine index)
      (%require-count "joystick-clear!" index)
      (%raise-if-error
        (%joystick-clear (%machine-token "joystick-clear!" machine) index))
      (if #f #f))

    (define (mouse-move! machine delta-x delta-y)
      (%require-integer "mouse-move!" delta-x)
      (%require-integer "mouse-move!" delta-y)
      (%raise-if-error
        (%mouse-move (%machine-token "mouse-move!" machine) delta-x delta-y))
      (if #f #f))

    (define (mouse-button! machine button pressed?)
      (%require-symbol "mouse-button!" button)
      (%require-boolean "mouse-button!" pressed?)
      (%raise-if-error
        (%mouse-button (%machine-token "mouse-button!" machine)
                       button pressed?))
      (if #f #f))

    (define (media-insert! machine type slot path)
      (%require-symbol "media-insert!" type)
      (%require-count "media-insert!" slot)
      (%require-string "media-insert!" path)
      (%raise-if-error
        (%media-insert (%machine-token "media-insert!" machine)
                       type slot path)))

    (define (media-eject! machine type slot)
      (%require-symbol "media-eject!" type)
      (%require-count "media-eject!" slot)
      (%raise-if-error
        (%media-eject (%machine-token "media-eject!" machine) type slot))
      (if #f #f))

    (define (media-flush! machine)
      (%raise-if-error
        (%media-flush (%machine-token "media-flush!" machine)))
      (if #f #f))

    (define (media-info machine type slot)
      (%require-symbol "media-info" type)
      (%require-count "media-info" slot)
      (%raise-if-error
        (%media-info (%machine-token "media-info" machine) type slot)))

    (define (create-hdd! machine type slot size)
      (%require-symbol "create-hdd!" type)
      (%require-count "create-hdd!" slot)
      (%require-symbol "create-hdd!" size)
      (%raise-if-error
        (%create-hdd (%machine-token "create-hdd!" machine)
                     type slot size)))

    (define (format-hdd! machine type slot . optional-table)
      (%require-symbol "format-hdd!" type)
      (%require-count "format-hdd!" slot)
      (let ((table
              (cond
                ((null? optional-table) 'pc98)
                ((null? (cdr optional-table)) (car optional-table))
                (else
                 (error "format-hdd!: expected three or four arguments"
                        'neetan/argument)))))
        (%require-symbol "format-hdd!" table)
        (%raise-if-error
          (%format-hdd (%machine-token "format-hdd!" machine)
                       type slot table))))

    (define (%require-tolerance who value)
      (if (and (real? value) (>= value 0) (<= value 1))
          (inexact value)
          (error (string-append who ": tolerance must be a real in [0, 1]")
                 'neetan/argument)))

    (define (screen-available? machine)
      (%screen-available?
        (%machine-token "screen-available?" machine)))

    (define (screen-size machine)
      (let ((size
              (%raise-if-error
                (%screen-size (%machine-token "screen-size" machine)))))
        (values (list-ref size 0) (list-ref size 1))))

    (define (screen-rgba machine)
      (%raise-if-error
        (%screen-rgba (%machine-token "screen-rgba" machine))))

    (define (screen-pixel machine x y)
      (%require-count "screen-pixel" x)
      (%require-count "screen-pixel" y)
      (let ((rgba
              (%raise-if-error
                (%screen-pixel
                  (%machine-token "screen-pixel" machine) x y))))
        (values (list-ref rgba 0) (list-ref rgba 1)
                (list-ref rgba 2) (list-ref rgba 3))))

    (define (screen-hash machine)
      (%raise-if-error
        (%screen-hash (%machine-token "screen-hash" machine))))

    (define (save-screenshot! machine artifact-path)
      (%require-string "save-screenshot!" artifact-path)
      (%raise-if-error
        (%save-screenshot
          (%machine-token "save-screenshot!" machine) artifact-path)))

    (define (screen-matches? machine expected-path . optional-options)
      (%require-string "screen-matches?" expected-path)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "screen-matches?" (car optional-options) '(tolerance)))
                 (else
                   (error "screen-matches?: expected two or three arguments"
                          'neetan/argument))))
             (tolerance
               (%require-tolerance
                 "screen-matches?" (alist-ref options 'tolerance 0.0))))
        (%raise-if-error
          (%screen-matches (%machine-token "screen-matches?" machine)
                           expected-path tolerance))))

    (define default-screen-wait-frames 1800)

    (define (wait-for-screen machine expected-path . optional-options)
      (%require-string "wait-for-screen" expected-path)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "wait-for-screen" (car optional-options)
                    '(tolerance frames ticks)))
                 (else
                   (error "wait-for-screen: expected two or three arguments"
                          'neetan/argument))))
             (missing (cons #f #f))
             (tolerance-value (alist-ref options 'tolerance missing))
             (frames-value (alist-ref options 'frames missing))
             (ticks-value (alist-ref options 'ticks missing))
             (tolerance
               (if (eq? tolerance-value missing)
                   0.0
                   (%require-tolerance "wait-for-screen" tolerance-value)))
             (frames
               (if (eq? frames-value missing)
                   default-screen-wait-frames
                   (%require-count "wait-for-screen" frames-value)))
             (ticks
               (if (eq? ticks-value missing)
                   (* frames default-maximum-ticks-per-frame)
                   (%require-count "wait-for-screen" ticks-value))))
        (%raise-if-error
          (%wait-for-screen
            (%machine-token "wait-for-screen" machine)
            expected-path tolerance frames ticks))))

    (define (screen-region-matches?
              machine expected-path x y width height . optional-options)
      (%require-string "screen-region-matches?" expected-path)
      (%require-count "screen-region-matches?" x)
      (%require-count "screen-region-matches?" y)
      (%require-count "screen-region-matches?" width)
      (%require-count "screen-region-matches?" height)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "screen-region-matches?" (car optional-options)
                    '(tolerance)))
                 (else
                   (error
                     "screen-region-matches?: expected six or seven arguments"
                     'neetan/argument))))
             (tolerance
               (%require-tolerance
                 "screen-region-matches?"
                 (alist-ref options 'tolerance 0.0))))
        (%raise-if-error
          (%screen-region-matches
            (%machine-token "screen-region-matches?" machine)
            expected-path x y width height tolerance))))))
