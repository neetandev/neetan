use device::i8259a_pic::I8259aPic;

#[test]
fn irq_mutations_report_only_request_state_changes() {
    let mut pic = I8259aPic::new_zeroed();

    assert!(pic.set_irq(3));
    assert!(!pic.set_irq(3));
    assert!(pic.clear_irq(3));
    assert!(!pic.clear_irq(3));

    assert!(pic.set_irq(12));
    assert!(!pic.set_irq(12));
    assert!(pic.clear_irq(12));
    assert!(!pic.clear_irq(12));
}

#[test]
fn icw_initialization_master() {
    let mut pic = I8259aPic::new_zeroed();

    // ICW1
    pic.write_port0(0, 0x11);
    assert_eq!(pic.chips[0].icw[0], 0x11);
    assert_eq!(pic.chips[0].write_icw, 1);

    // ICW2: vector base 0x08
    pic.write_port2(0, 0x08);
    assert_eq!(pic.chips[0].icw[1], 0x08);

    // ICW3: slave on IR7
    pic.write_port2(0, 0x80);
    assert_eq!(pic.chips[0].icw[2], 0x80);

    // ICW4: buffered master, normal EOI, x86 mode
    pic.write_port2(0, 0x1D);
    assert_eq!(pic.chips[0].icw[3], 0x1D);

    // ICW sequence complete
    assert_eq!(pic.chips[0].write_icw, 0);
    assert_eq!(pic.chips[0].imr, 0x00);
    assert_eq!(pic.chips[0].isr, 0);
    assert_eq!(pic.chips[0].irr, 0);
}

#[test]
fn icw_initialization_slave() {
    let mut pic = I8259aPic::new_zeroed();

    pic.write_port0(1, 0x11);
    pic.write_port2(1, 0x10); // vector base 0x10
    pic.write_port2(1, 0x07); // slave ID 7
    pic.write_port2(1, 0x09); // buffered slave

    assert_eq!(pic.chips[1].icw, [0x11, 0x10, 0x07, 0x09]);
    assert_eq!(pic.chips[1].write_icw, 0);
    assert_eq!(pic.chips[1].imr, 0x00);
}

#[test]
fn imr_read_write() {
    let mut pic = I8259aPic::new();

    // Master IMR
    pic.write_port2(0, 0x7D);
    assert_eq!(pic.read_port2(0), 0x7D);

    // Slave IMR
    pic.write_port2(1, 0x71);
    assert_eq!(pic.read_port2(1), 0x71);
}

#[test]
fn irq_acknowledge_eoi() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master with vector base 0x08, cascade on IR7
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00); // IMR = 0 (all unmasked)

    // Set IRQ 0
    pic.set_irq(0);
    assert_eq!(pic.chips[0].irr, 0x01);

    // Acknowledge
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x08); // base 0x08 + IRQ 0
    assert_eq!(pic.chips[0].irr, 0x00);
    assert_eq!(pic.chips[0].isr, 0x01);

    // Non-specific EOI
    pic.write_port0(0, 0x20);
    assert_eq!(pic.chips[0].isr, 0x00);
}

#[test]
fn read_irr_and_isr_via_ocw3() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    pic.set_irq(3);

    // Default reads IRR
    pic.write_port0(0, 0x08 | 0x02); // OCW3: RR=1, RIS=0 -> read IRR
    assert_eq!(pic.read_port0(0), 0x08); // IRQ 3 = bit 3

    let vector = pic.acknowledge();
    assert_eq!(vector, 0x0B); // 0x08 + 3

    // Switch to read ISR
    pic.write_port0(0, 0x08 | 0x02 | 0x01); // OCW3: RR=1, RIS=1 -> read ISR
    assert_eq!(pic.read_port0(0), 0x08); // IRQ 3 in-service

    // EOI
    pic.write_port0(0, 0x20);
    assert_eq!(pic.read_port0(0), 0x00); // ISR clear
}

#[test]
fn priority_resolution() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Set IRQ 2 and IRQ 5
    pic.set_irq(2);
    pic.set_irq(5);
    assert_eq!(pic.chips[0].irr, 0x24);

    // IRQ 2 has higher priority (lower number)
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x0A); // 0x08 + 2
    assert_eq!(pic.chips[0].irr, 0x20); // IRQ 5 still pending
    assert_eq!(pic.chips[0].isr, 0x04); // IRQ 2 in-service

    // IRQ 5 is blocked by higher-priority IRQ 2 in-service
    assert!(!pic.has_pending_irq());

    // EOI clears IRQ 2 from ISR
    pic.write_port0(0, 0x20);
    assert_eq!(pic.chips[0].isr, 0x00);

    // IRQ 5 now unblocked
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x0D); // 0x08 + 5
}

#[test]
fn master_slave_cascade() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master: vector base 0x08, slave on IR7
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80); // ICW3: slave on IR7
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00); // IMR = 0

    // Initialize slave: vector base 0x10, slave ID 7
    pic.write_port0(1, 0x11);
    pic.write_port2(1, 0x10);
    pic.write_port2(1, 0x07);
    pic.write_port2(1, 0x09);
    pic.write_port2(1, 0x00); // IMR = 0

    // Set IRQ 12 (slave IR4)
    pic.set_irq(12);
    assert_eq!(pic.chips[1].irr, 0x10); // bit 4

    // Acknowledge
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x14); // slave base 0x10 + 4

    // Master ISR bit 7 (cascade), slave ISR bit 4
    assert_eq!(pic.chips[0].isr, 0x80);
    assert_eq!(pic.chips[1].isr, 0x10);
    assert_eq!(pic.chips[0].irr & 0x80, 0x00); // master IRR cascade cleared
    assert_eq!(pic.chips[1].irr, 0x00); // slave IRR cleared

    // EOI to slave first, then master
    pic.write_port0(1, 0x20); // slave EOI
    assert_eq!(pic.chips[1].isr, 0x00);
    pic.write_port0(0, 0x20); // master EOI
    assert_eq!(pic.chips[0].isr, 0x00);
}

#[test]
fn masked_irq_does_not_fire() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x01); // IMR = 0x01 (IRQ 0 masked)

    pic.set_irq(0);
    assert_eq!(pic.chips[0].irr, 0x01);
    assert!(!pic.has_pending_irq());

    // Unmask
    pic.write_port2(0, 0x00);
    assert!(pic.has_pending_irq());
}

#[test]
fn specific_eoi() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Set IRQ 3 and acknowledge
    pic.set_irq(3);
    pic.acknowledge();
    assert_eq!(pic.chips[0].isr, 0x08);

    // Specific EOI for level 3
    pic.write_port0(0, 0x60 | 3); // specific EOI for level 3
    assert_eq!(pic.chips[0].isr, 0x00);
}

#[test]
fn rotate_on_eoi() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Set and acknowledge IRQ 2
    pic.set_irq(2);
    pic.acknowledge();
    assert_eq!(pic.chips[0].pry, 0); // priority base still 0

    // Rotate on non-specific EOI (0xA0 = R + EOI)
    pic.write_port0(0, 0xA0);
    assert_eq!(pic.chips[0].isr, 0x00);
    assert_eq!(pic.chips[0].pry, 3); // priority rotated past IRQ 2
}

#[test]
fn pc98_boot_defaults() {
    let pic = I8259aPic::new();

    assert_eq!(pic.chips[0].icw, [0x11, 0x08, 0x80, 0x1D]);
    assert_eq!(pic.chips[0].imr, 0x7D);
    assert_eq!(pic.chips[0].isr, 0);
    assert_eq!(pic.chips[0].irr, 0);

    assert_eq!(pic.chips[1].icw, [0x11, 0x10, 0x07, 0x09]);
    assert_eq!(pic.chips[1].imr, 0x71);
    assert_eq!(pic.chips[1].isr, 0);
    assert_eq!(pic.chips[1].irr, 0);
}

#[test]
fn icw_init_without_icw4() {
    let mut pic = I8259aPic::new_zeroed();

    // ICW1 with bit 0 = 0 (no ICW4 needed)
    pic.write_port0(0, 0x10);
    assert_eq!(pic.chips[0].icw[0], 0x10);
    assert_eq!(pic.chips[0].write_icw, 1);

    // ICW2: vector base 0x08
    pic.write_port2(0, 0x08);
    assert_eq!(pic.chips[0].icw[1], 0x08);

    // ICW3: cascade mask - sequence complete (3 + 0 = 3 ICWs)
    pic.write_port2(0, 0x80);
    assert_eq!(pic.chips[0].icw[2], 0x80);
    assert_eq!(pic.chips[0].write_icw, 0);

    // Next write_port2 should be OCW1 (IMR), NOT ICW4
    pic.write_port2(0, 0xFF);
    assert_eq!(pic.chips[0].imr, 0xFF);
    assert_eq!(pic.chips[0].icw[3], 0); // ICW4 untouched
}

#[test]
fn reinit_clears_isr() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Set IRQ 3, acknowledge -> ISR bit 3 set
    pic.set_irq(3);
    pic.acknowledge();
    assert_eq!(pic.chips[0].isr, 0x08);

    // Set IMR and IRR for verification
    pic.write_port2(0, 0xAA);
    pic.set_irq(5);

    // Reinitialize with ICW1
    pic.write_port0(0, 0x11);

    // ICW1 reinitialization clears the in-service latch.
    assert_eq!(pic.chips[0].isr, 0x00);
    // IMR, IRR, ocw3, pry cleared
    assert_eq!(pic.chips[0].imr, 0x00);
    assert_eq!(pic.chips[0].irr, 0x00);
    assert_eq!(pic.chips[0].ocw3, 0x00);
    assert_eq!(pic.chips[0].pry, 0);
}

#[test]
fn set_priority_command() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    assert_eq!(pic.chips[0].pry, 0);

    // Set priority: 0xC0 | 4 = 0xC4 (R=1, SL=1, EOI=0, L=4)
    // pry = (4 + 1) & 7 = 5
    pic.write_port0(0, 0xC4);
    assert_eq!(pic.chips[0].pry, 5);

    // Priority order now: 5, 6, 7, 0, 1, 2, 3, 4
    // IRQ 5 is highest priority, IRQ 4 is lowest
    pic.set_irq(4);
    pic.set_irq(5);

    let vector = pic.acknowledge();
    assert_eq!(vector, 0x08 + 5); // IRQ 5 scanned first

    // Reset via 0xC7: pry = (7+1)&7 = 0
    pic.write_port0(0, 0x20); // EOI first
    pic.write_port0(0, 0xC7);
    assert_eq!(pic.chips[0].pry, 0);
}

#[test]
fn rotate_on_specific_eoi() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Put IRQ 2 and IRQ 5 in-service. In special mask mode an in-service IRQ
    // stops blocking only after its mask bit is set.
    pic.set_irq(2);
    pic.acknowledge();
    pic.set_irq(5);
    pic.write_port0(0, 0x68); // enable SMM
    pic.write_port2(0, 0x04); // mask IRQ 2 while it remains in-service
    pic.acknowledge();
    pic.write_port0(0, 0x48); // disable SMM
    pic.write_port2(0, 0x00);
    assert_eq!(pic.chips[0].isr, 0x24); // bits 2 and 5

    // Rotate on specific EOI for level 2: 0xE0 | 2 = 0xE2
    pic.write_port0(0, 0xE2);

    // ISR bit 2 cleared, bit 5 remains
    assert_eq!(pic.chips[0].isr, 0x20);
    // Priority rotated: pry = (2 + 1) & 7 = 3
    assert_eq!(pic.chips[0].pry, 3);
}

#[test]
fn special_mask_mode() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Acknowledge IRQ 2 (put in-service)
    pic.set_irq(2);
    pic.acknowledge();
    assert_eq!(pic.chips[0].isr, 0x04);

    // Set IRQ 5 pending
    pic.set_irq(5);

    // Without SMM, IRQ 5 blocked by IRQ 2 in-service
    assert!(!pic.has_pending_irq());

    // Enable special mask mode: OCW3 = 0x68 (ESMM=1, SMM=1, bit3=1)
    pic.write_port0(0, 0x68);
    assert_ne!(pic.chips[0].ocw3 & 0x20, 0);

    // SMM alone does not unblock IRQ 5 while IRQ 2 is still unmasked.
    assert!(!pic.has_pending_irq());

    // Masking the in-service IRQ 2 under SMM lets lower-priority IRQ 5 fire.
    pic.write_port2(0, 0x04);
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x08 + 5);
    pic.write_port2(0, 0x00);

    // Disable SMM: OCW3 = 0x48 (ESMM=1, SMM=0)
    pic.write_port0(0, 0x48);
    assert_eq!(pic.chips[0].ocw3 & 0x20, 0);
}

#[test]
fn simultaneous_master_and_slave_irqs() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master: vector base 0x08, slave on IR7
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Initialize slave: vector base 0x10, slave ID 7
    pic.write_port0(1, 0x11);
    pic.write_port2(1, 0x10);
    pic.write_port2(1, 0x07);
    pic.write_port2(1, 0x09);
    pic.write_port2(1, 0x00);

    // Set master IRQ 3 AND slave IRQ 9 (slave IR1)
    pic.set_irq(3);
    pic.set_irq(9);

    // Master IRQ 3 has higher priority than cascade IR7
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x08 + 3);
    assert_eq!(pic.chips[0].isr, 0x08);

    // Slave IRQ 9 blocked by master ISR
    assert!(!pic.has_pending_irq());

    // EOI master, now slave fires via cascade
    pic.write_port0(0, 0x20);
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10 + 1);
}

#[test]
fn slave_irq_priority_resolution() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master: slave on IR7
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Initialize slave
    pic.write_port0(1, 0x11);
    pic.write_port2(1, 0x10);
    pic.write_port2(1, 0x07);
    pic.write_port2(1, 0x09);
    pic.write_port2(1, 0x00);

    // Set slave IRQ 10 (IR2), IRQ 12 (IR4), IRQ 14 (IR6)
    pic.set_irq(10);
    pic.set_irq(12);
    pic.set_irq(14);
    assert_eq!(pic.chips[1].irr, 0x54); // bits 2, 4, 6

    // IRQ 10 (slave IR2) fires first (lowest numbered)
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10 + 2);
    assert_eq!(pic.chips[1].isr, 0x04);

    // IRQ 12 blocked by slave ISR
    assert!(!pic.has_pending_irq());

    // EOI slave + master cascade
    pic.write_port0(1, 0x20);
    pic.write_port0(0, 0x20);

    // IRQ 12 fires next
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10 + 4);
}

#[test]
fn slave_irq_blocked_by_master_isr() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master: slave on IR7
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Initialize slave
    pic.write_port0(1, 0x11);
    pic.write_port2(1, 0x10);
    pic.write_port2(1, 0x07);
    pic.write_port2(1, 0x09);
    pic.write_port2(1, 0x00);

    // Put master IRQ 3 in-service
    pic.set_irq(3);
    pic.acknowledge();
    assert_eq!(pic.chips[0].isr, 0x08);

    // Set slave IRQ 8 (slave IR0) - cascade on IR7
    pic.set_irq(8);

    // Cascade IR7 is lower priority than ISR'd IRQ 3 -> blocked
    assert!(!pic.has_pending_irq());

    // EOI for IRQ 3
    pic.write_port0(0, 0x20);

    // Now slave IRQ fires
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10);
}

#[test]
fn non_specific_eoi_with_rotated_priority() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Set priority: 0xC0 | 3 -> pry = (3+1)&7 = 4
    pic.write_port0(0, 0xC3);
    assert_eq!(pic.chips[0].pry, 4);

    // Manually put IRQ 5 and IRQ 1 in ISR
    pic.chips[0].isr = 0x22; // bits 1 and 5

    // Non-specific EOI: scans from pry=4
    // Scan: bit 4=0, bit 5=1 -> clears bit 5
    pic.write_port0(0, 0x20);
    assert_eq!(pic.chips[0].isr, 0x02);

    // Another non-specific EOI: scan from pry=4
    // Scan: 4=0, 5=0, 6=0, 7=0, 0=0, 1=1 -> clears bit 1
    pic.write_port0(0, 0x20);
    assert_eq!(pic.chips[0].isr, 0x00);
}

#[test]
fn clear_irq_while_in_service() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Set IRQ 4, acknowledge
    pic.set_irq(4);
    pic.acknowledge();
    assert_eq!(pic.chips[0].isr, 0x10);
    assert_eq!(pic.chips[0].irr, 0x00);

    // Clear IRQ 4 - ISR must be unaffected
    pic.clear_irq(4);
    assert_eq!(pic.chips[0].isr, 0x10);

    // Re-raise then clear
    pic.set_irq(4);
    assert_eq!(pic.chips[0].irr, 0x10);
    pic.clear_irq(4);
    assert_eq!(pic.chips[0].irr, 0x00);
    assert_eq!(pic.chips[0].isr, 0x10); // still in-service
}

#[test]
fn mask_irq_while_in_service() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);

    // Set IRQ 1, acknowledge
    pic.set_irq(1);
    pic.acknowledge();
    assert_eq!(pic.chips[0].isr, 0x02);

    // Mask IRQ 1 - ISR still has it
    pic.write_port2(0, 0x02);
    assert_eq!(pic.chips[0].isr, 0x02);

    // EOI clears IRQ 1 from ISR
    pic.write_port0(0, 0x20);
    assert_eq!(pic.chips[0].isr, 0x00);

    // Re-raise IRQ 1 - blocked by mask
    pic.set_irq(1);
    assert!(!pic.has_pending_irq());

    // Unmask IRQ 1
    pic.write_port2(0, 0x00);
    assert!(pic.has_pending_irq());
}

// --- Helper to initialize a standard PC-98 master+slave PIC pair ---

fn init_master(pic: &mut I8259aPic) {
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x80);
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00);
}

fn init_slave(pic: &mut I8259aPic) {
    pic.write_port0(1, 0x11);
    pic.write_port2(1, 0x10);
    pic.write_port2(1, 0x07);
    pic.write_port2(1, 0x09);
    pic.write_port2(1, 0x00);
}

#[test]
fn ocw2_rotate_only_no_eoi() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);

    // Acknowledge IRQ 2 -> ISR bit 2 set
    pic.set_irq(2);
    pic.acknowledge();
    assert_eq!(pic.chips[0].isr, 0x04);
    assert_eq!(pic.chips[0].pry, 0);

    // OCW2 0x80: R=1, SL=0, EOI=0 (rotate only, no EOI)
    // Non-specific scan from pry=0 finds ISR bit 2 -> pry=(2+1)&7=3
    // EOI bit is NOT set, so ISR is NOT cleared
    pic.write_port0(0, 0x80);
    assert_eq!(pic.chips[0].pry, 3);
    assert_eq!(pic.chips[0].isr, 0x04);
}

#[test]
fn non_specific_eoi_with_empty_isr() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);

    // ISR is empty
    assert_eq!(pic.chips[0].isr, 0x00);

    // Non-specific EOI with empty ISR: should be a no-op
    pic.write_port0(0, 0x20);
    assert_eq!(pic.chips[0].isr, 0x00);
    assert_eq!(pic.chips[0].pry, 0);

    // Rotate + non-specific EOI (0xA0) with empty ISR: also no-op
    pic.write_port0(0, 0xA0);
    assert_eq!(pic.chips[0].isr, 0x00);
    assert_eq!(pic.chips[0].pry, 0);
}

#[test]
fn specific_eoi_for_wrong_level() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);

    // Acknowledge IRQ 3 -> ISR bit 3
    pic.set_irq(3);
    pic.acknowledge();
    assert_eq!(pic.chips[0].isr, 0x08);

    // Specific EOI for level 5 (not in-service): ISR unchanged
    pic.write_port0(0, 0x60 | 5);
    assert_eq!(pic.chips[0].isr, 0x08);

    // Specific EOI for level 0 (not in-service): ISR unchanged
    pic.write_port0(0, 0x60);
    assert_eq!(pic.chips[0].isr, 0x08);

    // Specific EOI for level 3 (correct): ISR cleared
    pic.write_port0(0, 0x60 | 3);
    assert_eq!(pic.chips[0].isr, 0x00);
}

#[test]
fn all_master_irq_vectors() {
    let mut pic = I8259aPic::new_zeroed();

    // Initialize master with NO cascade (ICW3=0x00)
    pic.write_port0(0, 0x11);
    pic.write_port2(0, 0x08);
    pic.write_port2(0, 0x00); // ICW3: no cascade bits
    pic.write_port2(0, 0x1D);
    pic.write_port2(0, 0x00); // IMR = 0

    for irq in 0..8u8 {
        pic.set_irq(irq);
        assert!(pic.has_pending_irq(), "IRQ {irq} should be pending");
        let vector = pic.acknowledge();
        assert_eq!(vector, 0x08 + irq, "IRQ {irq} vector mismatch");
        pic.write_port0(0, 0x20); // EOI
    }
}

#[test]
fn all_slave_irq_vectors() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);
    init_slave(&mut pic);

    for irq in 8..16u8 {
        pic.set_irq(irq);
        assert!(pic.has_pending_irq(), "IRQ {irq} should be pending");
        let vector = pic.acknowledge();
        assert_eq!(vector, 0x10 + (irq - 8), "IRQ {irq} vector mismatch");
        pic.write_port0(1, 0x20); // slave EOI
        pic.write_port0(0, 0x20); // master EOI
    }
}

#[test]
fn priority_wrap_around() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);

    // Set pry=6 via 0xC5 (R=1, SL=1, EOI=0, L=5 -> pry=(5+1)&7=6)
    pic.write_port0(0, 0xC5);
    assert_eq!(pic.chips[0].pry, 6);

    // Priority order now: 6,7,0,1,2,3,4,5 (IRQ 5 is lowest, IRQ 6 is highest)
    pic.set_irq(5);
    pic.set_irq(6);

    // IRQ 6 fires first (highest priority)
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x08 + 6);
    pic.write_port0(0, 0x20);

    // IRQ 5 fires next
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x08 + 5);
    pic.write_port0(0, 0x20);
}

#[test]
fn ocw3_ris_sticky_when_rr_zero() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);

    // Set RIS=1 via OCW3 with RR=1: 0x0B (bit3=1, RR=1, RIS=1)
    pic.write_port0(0, 0x0B);
    assert_eq!(pic.chips[0].ocw3 & 0x01, 1);

    // Write OCW3 with RR=0: 0x08. RIS should be preserved from old
    pic.write_port0(0, 0x08);
    assert_eq!(pic.chips[0].ocw3 & 0x01, 1);

    // Explicitly set RIS=0 with RR=1: 0x0A
    pic.write_port0(0, 0x0A);
    assert_eq!(pic.chips[0].ocw3 & 0x01, 0);

    // Write OCW3 with RR=0: 0x09 (bit 0=1, but RR=0 so RIS stays from old)
    pic.write_port0(0, 0x09);
    assert_eq!(pic.chips[0].ocw3 & 0x01, 0);
}

#[test]
fn ocw3_smm_sticky_when_esmm_zero() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);

    // Enable SMM: 0x68 (ESMM=1, SMM=1, bit3=1)
    pic.write_port0(0, 0x68);
    assert_eq!(pic.chips[0].ocw3 & 0x20, 0x20);

    // Write OCW3 with ESMM=0: 0x08. SMM should be preserved
    pic.write_port0(0, 0x08);
    assert_eq!(pic.chips[0].ocw3 & 0x20, 0x20);

    // Disable SMM: 0x48 (ESMM=1, SMM=0)
    pic.write_port0(0, 0x48);
    assert_eq!(pic.chips[0].ocw3 & 0x20, 0);

    // Verify it stays off when ESMM=0
    pic.write_port0(0, 0x08);
    assert_eq!(pic.chips[0].ocw3 & 0x20, 0);
}

#[test]
fn reinit_during_icw_sequence() {
    let mut pic = I8259aPic::new_zeroed();

    // Start ICW sequence
    pic.write_port0(0, 0x11);
    assert_eq!(pic.chips[0].write_icw, 1);

    // Write ICW2
    pic.write_port2(0, 0x08);
    assert_eq!(pic.chips[0].write_icw, 2);

    // Mid-sequence: write ICW1 again (restart)
    pic.write_port0(0, 0x11);
    assert_eq!(pic.chips[0].write_icw, 1);

    // Complete new sequence with different vector base
    pic.write_port2(0, 0x20); // ICW2: new base 0x20
    pic.write_port2(0, 0x00); // ICW3: no cascade
    pic.write_port2(0, 0x01); // ICW4: x86 mode

    assert_eq!(pic.chips[0].write_icw, 0);
    assert_eq!(pic.chips[0].icw[1], 0x20);
    assert_eq!(pic.chips[0].icw[2], 0x00);
    assert_eq!(pic.chips[0].icw[3], 0x01);

    // Verify the new vector base works
    pic.write_port2(0, 0x00); // IMR = 0
    pic.set_irq(0);
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x20); // new base
}

#[test]
fn slave_irq_blocked_by_own_isr() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);
    init_slave(&mut pic);

    // Acknowledge slave IRQ 8 (IR0) -> sets slave ISR bit 0
    pic.set_irq(8);
    pic.acknowledge();
    assert_eq!(pic.chips[1].isr, 0x01);
    assert_eq!(pic.chips[0].isr, 0x80); // cascade bit

    // Raise IRQ 10 (slave IR2): blocked by slave ISR bit 0
    pic.set_irq(10);
    assert!(!pic.has_pending_irq());

    // EOI slave + master
    pic.write_port0(1, 0x20);
    pic.write_port0(0, 0x20);

    // Now IRQ 10 fires
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10 + 2);
}

#[test]
fn multiple_slave_irqs_sequential() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);
    init_slave(&mut pic);

    // Set slave IRQ 9 (IR1), IRQ 11 (IR3), IRQ 13 (IR5) simultaneously
    pic.set_irq(9);
    pic.set_irq(11);
    pic.set_irq(13);
    assert_eq!(pic.chips[1].irr, 0x2A); // bits 1, 3, 5

    // IRQ 9 fires first (lowest numbered)
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10 + 1);
    pic.write_port0(1, 0x20);
    pic.write_port0(0, 0x20);

    // IRQ 11 fires next
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10 + 3);
    pic.write_port0(1, 0x20);
    pic.write_port0(0, 0x20);

    // IRQ 13 fires last
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10 + 5);
    pic.write_port0(1, 0x20);
    pic.write_port0(0, 0x20);

    assert!(!pic.has_pending_irq());
}

#[test]
fn master_irq_at_cascade_position_without_slave() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);
    init_slave(&mut pic);

    // Raise master IRQ 7 directly (cascade bit position, no slave IRQs)
    pic.set_irq(7);

    // Priority scan hits bit 7 -> enters cascade branch -> sir==0 -> returns None
    assert!(!pic.has_pending_irq());

    // But if a slave IRQ is also pending, the cascade fires
    pic.set_irq(8);
    assert!(pic.has_pending_irq());
    let vector = pic.acknowledge();
    assert_eq!(vector, 0x10); // slave IR0
}

#[test]
fn canonical_slave_eoi_pattern() {
    let mut pic = I8259aPic::new_zeroed();
    init_master(&mut pic);
    init_slave(&mut pic);

    // Acknowledge slave IRQ 12 (IR4)
    pic.set_irq(12);
    pic.acknowledge();
    assert_eq!(pic.chips[1].isr, 0x10);
    assert_eq!(pic.chips[0].isr, 0x80);

    // Reproduce the undoc98 slave EOI pattern (io_pic.txt lines 113-125):
    // 1. Send EOI to slave
    pic.write_port0(1, 0x20);
    assert_eq!(pic.chips[1].isr, 0x00);

    // 2. Set OCW3 to read ISR on slave (0x0B = bit3=1, RR=1, RIS=1)
    pic.write_port0(1, 0x0B);

    // 3. Read slave ISR
    let slave_isr = pic.read_port0(1);
    assert_eq!(slave_isr, 0x00); // no more in-service on slave

    // 4. Since slave ISR == 0, send EOI to master
    pic.write_port0(0, 0x20);
    assert_eq!(pic.chips[0].isr, 0x00);

    // Both master and slave are now fully cleared
    assert!(!pic.has_pending_irq());
}

#[test]
fn acknowledgement_reports_master_slave_and_spurious_lines() {
    let mut master = I8259aPic::new_zeroed();
    init_master(&mut master);
    init_slave(&mut master);
    master.set_irq(3);
    let acknowledge = master.acknowledge_with_line();
    assert_eq!(acknowledge.line, Some(3));
    assert_eq!(acknowledge.vector, 0x0B);

    let mut slave = I8259aPic::new_zeroed();
    init_master(&mut slave);
    init_slave(&mut slave);
    slave.set_irq(12);
    let acknowledge = slave.acknowledge_with_line();
    assert_eq!(acknowledge.line, Some(12));
    assert_eq!(acknowledge.vector, 0x14);

    let mut spurious = I8259aPic::new_zeroed();
    init_master(&mut spurious);
    init_slave(&mut spurious);
    let acknowledge = spurious.acknowledge_with_line();
    assert_eq!(acknowledge.line, None);
    assert_eq!(acknowledge.vector, 0x0F);
}
