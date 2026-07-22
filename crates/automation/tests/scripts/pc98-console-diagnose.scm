;; End-to-end coverage of the semantic PC-98 HLE DOS console trace events,
;; proven entirely from structured trace values, processor snapshots, decoded
;; text cells, and bounded artifacts.
;;
;; A synthetic COM program is injected into free conventional memory and run
;; under the booted HLE DOS. It:
;;   clears part of a line with ESC [ K and scrolls with ESC [ M,
;;   emits ESC [ 7 m (SGR reverse video) then a normal glyph,
;;   emits the two-byte PC-98 graphic 0x86 0x5F,
;;   writes a recognizable buffer through INT 21h AH=40h,
;;   installs an INT 29h -> IRET hook through INT 21h AH=25h,
;;   repeats the AH=40h write, which the hook now suppresses,
;;   then issues an AH=02h write that the hook also suppresses.
;; The clear and scroll run first so the later glyph rows stay in place for
;; the decoded text-surface cross-check.
(import (scheme base)
        (neetan automation 1)
        (neetan trace 1)
        (neetan inspect 1)
        (neetan mutate 1)
        (neetan test 1))

;; The synthetic program, assembled by hand. Loaded at 0x3000:0x0100, so its
;; linear base is 0x30100. "DIAG" sits at offset 0x0160 (linear 0x30160) and
;; the IRET byte at offset 0x0164 (linear 0x30164).
(define program
  #u8(#xB4 #x02             ; MOV AH,02h
      #xB2 #x1B #xCD #x21   ; DL=ESC  INT 21h
      #xB2 #x5B #xCD #x21   ; DL='['  INT 21h
      #xB2 #x4B #xCD #x21   ; DL='K'  INT 21h (clear line from cursor)
      #xB2 #x1B #xCD #x21   ; DL=ESC  INT 21h
      #xB2 #x5B #xCD #x21   ; DL='['  INT 21h
      #xB2 #x4D #xCD #x21   ; DL='M'  INT 21h (delete line, scrolls up)
      #xB2 #x1B #xCD #x21   ; DL=ESC  INT 21h
      #xB2 #x5B #xCD #x21   ; DL='['  INT 21h
      #xB2 #x37 #xCD #x21   ; DL='7'  INT 21h
      #xB2 #x6D #xCD #x21   ; DL='m'  INT 21h
      #xB2 #x58 #xCD #x21   ; DL='X'  INT 21h
      #xB2 #x86 #xCD #x21   ; DL=0x86 INT 21h (Shift-JIS lead)
      #xB2 #x5F #xCD #x21   ; DL=0x5F INT 21h (Shift-JIS trail)
      #xBB #x01 #x00        ; MOV BX,0001h (stdout handle)
      #xB9 #x04 #x00        ; MOV CX,0004h (length)
      #xBA #x60 #x01        ; MOV DX,0160h ("DIAG")
      #xB4 #x40 #xCD #x21   ; AH=40h INT 21h (write, console route)
      #xB8 #x29 #x25        ; MOV AX,2529h (AH=25h AL=29h)
      #xBA #x64 #x01        ; MOV DX,0164h (IRET byte)
      #xCD #x21             ; INT 21h (set INT 29h vector)
      #xBB #x01 #x00        ; MOV BX,0001h (stdout handle)
      #xB9 #x04 #x00        ; MOV CX,0004h (length)
      #xBA #x60 #x01        ; MOV DX,0160h ("DIAG")
      #xB4 #x40 #xCD #x21   ; AH=40h INT 21h (write, suppressed by the hook)
      #xB4 #x02             ; MOV AH,02h
      #xB2 #x59 #xCD #x21   ; DL='Y' INT 21h (suppressed by the hook)
      #xEB #xFE             ; JMP $
      #x44 #x49 #x41 #x47   ; "DIAG"
      #xCF))               ; IRET

(define program-segment #x3000)
(define program-linear #x30100)
(define diag-linear #x30160)
(define iret-linear #x30164)

;; Returns the value for key in an alist, or #f.
(define (field key alist)
  (let ((entry (assq key alist)))
    (and entry (cdr entry))))

(define (data-of event) (field 'data event))
(define (device-of event) (field 'device (data-of event)))
(define (action-of event) (field 'action (data-of event)))
(define (fields-of event) (field 'fields (data-of event)))

;; Returns the first item for which predicate holds, or #f.
(define (find-first predicate items)
  (cond ((null? items) #f)
        ((predicate (car items)) (car items))
        (else (find-first predicate (cdr items)))))

(define (device-event? event device action)
  (and (eq? (device-of event) device)
       (eq? (action-of event) action)))

;; Returns #t when thunk raises an error carrying the given neetan symbol.
(define (raises? symbol thunk)
  (guard (condition
          (#t (and (error-object? condition)
                   (memq symbol (error-object-irritants condition))
                   #t)))
    (thunk)
    #f))

;; Injects the program and points the guest at its entry.
(define (install-program! machine)
  (memory-write-bytevector! machine 'cpu.main.memory program-linear program)
  (register-set! machine 'cpu.main 'cs program-segment)
  (register-set! machine 'cpu.main 'ds program-segment)
  (register-set! machine 'cpu.main 'es program-segment)
  (register-set! machine 'cpu.main 'ss program-segment)
  (register-set! machine 'cpu.main 'sp #xFFFE)
  (register-set! machine 'cpu.main 'ip #x0100))

(test-suite "PC-98 console diagnose"

  (with-machine (machine '((target . pc98) (model . pc9801vm)))
    (test-case "semantic console events proven from structured trace values"
      (run-frames! machine 30)
      (install-program! machine)
      (trace-start! machine '((class . device)))
      (run-ticks! machine 2000000)
      (trace-stop! machine)

      ;; save-trace! writes the buffered events as Scheme data and is
      ;; non-consuming, so the same events are still drainable below.
      (check-true (> (field 'bytes (save-trace! machine "diagnose-trace.scm")) 0))

      (let ((events (trace-drain! machine)))
        (check-true (> (length events) 0))

        ;; SGR reverse video is honoured, not ignored. The escape
        ;; carries command sgr with parameter 7 and changes the attribute.
        (let ((escape (find-first
                        (lambda (event)
                          (and (device-event? event 'neetan.dos.console 'escape)
                               (eq? 'sgr (field 'command (fields-of event)))))
                        events)))
          (check-true (pair? escape))
          (check-equal 'sgr (field 'command (fields-of escape)))
          ;; Parameters are a list of exact integers, so the workflow shape
          ;; (member 7 parameters) from the requirements works directly.
          (check-equal '(7) (field 'parameters (fields-of escape)))
          (check-true (and (member 7 (field 'parameters (fields-of escape))) #t))
          (check-true (not (= (field 'attribute-before (fields-of escape))
                              (field 'attribute-after (fields-of escape))))))

        ;; The two-byte graphic 0x86 0x5F occupies one cell and
        ;; advances the cursor one column, not two. The trailing byte event
        ;; consumes the pending Shift-JIS lead and moves the cursor by one.
        (let ((trail (find-first
                       (lambda (event)
                         (and (device-event? event 'neetan.dos.console 'byte)
                              (equal? 95 (field 'byte (fields-of event)))))
                       events)))
          (check-true (pair? trail))
          (check-equal 134 (field 'pending-shift-jis-lead-before (fields-of trail)))
          (check-false (field 'pending-shift-jis-lead-after (fields-of trail)))
          (check-equal 1 (- (field 'cursor-column-after (fields-of trail))
                            (field 'cursor-column-before (fields-of trail)))))
        (let ((cell (find-first
                      (lambda (event)
                        (and (device-event? event 'neetan.dos.console 'cell-write)
                             (equal? 11072 (field 'jis (fields-of event)))))
                      events)))
          (check-true (pair? cell))
          (check-equal 1 (field 'display-width (fields-of cell)))
          ;; Cross-check the decoded surface: the traced cell agrees with a
          ;; side-effect-free text-cell read of the same position.
          (let ((decoded (text-cell machine 'display.main
                                    (field 'row (fields-of cell))
                                    (field 'column (fields-of cell)))))
            (check-equal 11072 (field 'raw-jis decoded))
            (check-equal (field 'attribute (fields-of cell))
                         (field 'attribute decoded))))

        ;; An INT 29h -> IRET hook suppresses console output. The
        ;; vector-set event records the linear target, memory confirms the IRET
        ;; opcode there, and the following write is routed to suppressed.
        (let ((vector (find-first
                        (lambda (event)
                          (device-event? event 'neetan.dos.vector 'set))
                        events)))
          (check-true (pair? vector))
          (check-equal 41 (field 'vector (fields-of vector)))
          (check-equal iret-linear (field 'linear-address (fields-of vector)))
          (check-equal #xCF
                       (memory-peek-unsigned machine 'cpu.main.memory
                                             iret-linear 1 'little)))
        ;; The post-hook AH=40h write reports the suppressed route and the
        ;; explicit IRET-hook reason, matching the required workflow shape.
        (let ((suppressed-write (find-first
                                  (lambda (event)
                                    (and (device-event? event 'neetan.dos.stdout 'write)
                                         (eq? 'int21.40 (field 'source (fields-of event)))
                                         (eq? 'suppressed (field 'route (fields-of event)))))
                                  events)))
          (check-true (pair? suppressed-write))
          (check-equal 1 (field 'handle (fields-of suppressed-write)))
          (check-equal 'int29-iret-hook
                       (field 'suppression-reason (fields-of suppressed-write)))
          (check-equal #u8(68 73 65 71) (field 'bytes (fields-of suppressed-write))))
        ;; The AH=02h write is suppressed too. It takes no handle argument, so
        ;; the event reports the handle as #f.
        (let ((suppressed-byte (find-first
                                 (lambda (event)
                                   (and (device-event? event 'neetan.dos.stdout 'write)
                                        (eq? 'int21.02 (field 'source (fields-of event)))
                                        (eq? 'suppressed (field 'route (fields-of event)))))
                                 events)))
          (check-true (pair? suppressed-byte))
          (check-false (field 'handle (fields-of suppressed-byte)))
          (check-equal 'int29-iret-hook
                       (field 'suppression-reason (fields-of suppressed-byte)))
          (check-equal #u8(89) (field 'bytes (fields-of suppressed-byte))))

        ;; The pre-hook AH=40h write carries its handle, buffer address, count,
        ;; bytes, and the console route.
        (let ((write (find-first
                       (lambda (event)
                         (and (device-event? event 'neetan.dos.stdout 'write)
                              (eq? 'int21.40 (field 'source (fields-of event)))
                              (eq? 'console (field 'route (fields-of event)))))
                       events)))
          (check-true (pair? write))
          (check-equal 1 (field 'handle (fields-of write)))
          (check-equal diag-linear (field 'buffer-address (fields-of write)))
          (check-equal 4 (field 'requested-count (fields-of write)))
          (check-equal #u8(68 73 65 71) (field 'bytes (fields-of write))))

        ;; Clear and scroll report the affected region as structured events,
        ;; distinguishing text-plane operations from pixels drawn black.
        (let ((clear (find-first
                       (lambda (event)
                         (device-event? event 'neetan.dos.console 'clear))
                       events)))
          (check-true (pair? clear))
          (check-true (>= (field 'region-bottom (fields-of clear))
                          (field 'region-top (fields-of clear))))
          (check-true (> (field 'count (fields-of clear)) 0)))
        (let ((scroll (find-first
                        (lambda (event)
                          (device-event? event 'neetan.dos.console 'scroll))
                        events)))
          (check-true (pair? scroll))
          (check-true (>= (field 'region-bottom (fields-of scroll))
                          (field 'region-top (fields-of scroll))))
          (check-equal 1 (field 'count (fields-of scroll)))
          (check-equal 1 (field 'direction (fields-of scroll)))))

      ;; Bounded artifacts land under the artifact root and a `..` path is
      ;; rejected before anything is written.
      (check-true (> (field 'bytes
                            (save-memory! machine 'cpu.main.memory iret-linear 1
                                          "diagnose-iret.bin"))
                     0))
      (check-true (>= (field 'bytes
                             (save-text-screen! machine 'display.main
                                                "diagnose-text.txt"))
                      0))
      (check-true (raises? 'neetan/path-escape
                           (lambda ()
                             (save-memory! machine 'cpu.main.memory iret-linear 1
                                           "../escape.bin"))))
      (check-true (raises? 'neetan/path-escape
                           (lambda ()
                             (save-memory! machine 'cpu.main.memory iret-linear 1
                                           "/tmp/escape.bin"))))))

  (with-machine (machine '((target . pc98) (model . pc9801vm)))
    (test-case "a #f constraint matches a falseable field with no value"
      (run-frames! machine 30)
      (install-program! machine)
      ;; The AH=40h write routes to the console before the INT 29h hook is
      ;; installed, so its falseable suppression-reason field is absent and a
      ;; #f constraint selects it.
      (let ((event (wait-for-event machine
                     '((class . device)
                       (data . ((device . neetan.dos.stdout) (action . write)
                                (fields . ((source . int21.40)
                                           (suppression-reason . #f))))))
                     '((frames . 5)))))
        (check-true (pair? event))
        (check-equal 'console (field 'route (fields-of event)))
        (check-false (field 'suppression-reason (fields-of event)))))

    (test-case "entry snapshot exposes the syscall registers"
      ;; The program continues from the matched write; the vector set follows.
      ;; The snapshot is taken at HLE dispatch entry, so it exposes the guest
      ;; AH=25h AL=29h and DS:DX arguments of the set-vector call.
      ;; The nested `fields` block filters on the provider-specific vector field,
      ;; so only the INT 29h (vector 41) set is matched.
      (let* ((event (wait-for-event machine
                      '((class . device)
                        (data . ((device . neetan.dos.vector) (action . set)
                                 (fields . ((vector . 41))))))
                      '((snapshot . (cpu.main)) (frames . 5))))
             (registers (field 'cpu.main (field 'snapshot event))))
        (check-true (pair? event))
        (check-true (pair? registers))
        (check-equal #x2529 (field 'ax registers))
        (check-equal #x0164 (field 'dx registers))
        (check-equal program-segment (field 'ds registers))))

    (test-case "call events accept provider field filters and carry snapshots"
      ;; The next AH=40h call is the suppressed post-hook write. The provider
      ;; field filter selects it by function number, and the boundary call
      ;; event itself carries the entry snapshot, so BX, CX, and DS:DX of the
      ;; write are readable regardless of what the handler clobbers.
      (let* ((event (wait-for-event machine
                      '((class . call)
                        (data . ((provider . neetan.dos)
                                 (phase . enter)
                                 (fields . ((function . 64))))))
                      '((snapshot . (cpu.main)) (frames . 5))))
             (registers (field 'cpu.main (field 'snapshot event))))
        (check-true (pair? event))
        (check-equal 64 (field 'function (fields-of event)))
        (check-true (pair? registers))
        (check-equal 1 (field 'bx registers))
        (check-equal 4 (field 'cx registers))
        (check-equal #x0160 (field 'dx registers))
        (check-equal program-segment (field 'ds registers))))

    (test-case "non-HLE events carry no snapshot"
      ;; A presentation event is not an HLE dispatch, so its snapshot is #f even
      ;; when a snapshot processor is requested.
      (let ((event (wait-for-event machine
                     '((class . presentation))
                     '((snapshot . (cpu.main)) (frames . 3)))))
        (check-true (pair? event))
        (check-false (field 'snapshot event))))))
