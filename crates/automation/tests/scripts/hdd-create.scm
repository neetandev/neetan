;; Script-driven in-memory HDD creation, formatting, and hard-reset survival.
(import (scheme base) (neetan automation 1) (neetan test 1))

(test-suite "PC-98 in-memory HDD"
  (with-machine (machine '((model . PC9801VM)))
    (test-case "create-hdd! mounts a blank RAM disk"
      (create-hdd! machine 'hdd 0 'sasi-40)
      (let ((info (media-info machine 'hdd 0)))
        (check-true (pair? info))
        (check-equal 'hdd (alist-ref info 'type))))

    (test-case "format-hdd! succeeds with a PC-98 partition table"
      (check-true (format-hdd! machine 'hdd 0 'pc98)))

    (test-case "the formatted HDD survives a hard reset"
      (reset! machine 'hard)
      (check-true (pair? (media-info machine 'hdd 0))))))
