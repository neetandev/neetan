from __future__ import annotations

import re
import sys
from pathlib import Path

HELPER_METHODS = {
    'access_to_be_redone', 'set_ftu_const', 'update_user_super', 'update_interrupt',
    'start_interrupt_vector_lookup', 'end_interrupt_vector_lookup', 'abort_access',
    'do_post_run', 'step_movem', 'step_movem_predec', 'debugger_exception_hook',
    'debugger_wait_hook',
    'alu_add', 'alu_add8', 'alu_addc', 'alu_addx', 'alu_addx8', 'alu_and', 'alu_andx',
    'alu_and8', 'alu_and8x', 'alu_or', 'alu_or8', 'alu_eor', 'alu_eor8', 'alu_ext',
    'alu_not', 'alu_not8', 'alu_sub', 'alu_sub8', 'alu_subc', 'alu_subx', 'alu_subx8',
    'alu_abcd8', 'alu_sbcd8', 'alu_sla0', 'alu_sla1', 'alu_over', 'alu_asl',
    'alu_asl8', 'alu_asl32', 'alu_asr', 'alu_asr8', 'alu_asr32', 'alu_lsl',
    'alu_lsl8', 'alu_lsl32', 'alu_lsr', 'alu_lsr8', 'alu_lsr32', 'alu_rol',
    'alu_rol8', 'alu_rol32', 'alu_ror', 'alu_ror8', 'alu_ror32', 'alu_roxl',
    'alu_roxl8', 'alu_roxl32', 'alu_roxr', 'alu_roxr8', 'alu_roxr32',
    'alu_roxr32ms', 'alu_roxr32mu', 'sr_z', 'sr_nz_u', 'sr_nzvc', 'sr_nzvc_u',
    'sr_xnzvc', 'sr_xnzvc_u', 'map_sp',
}

NOOP_LINES = {
    'debugger_wait_hook();': 'self.debugger_wait_hook();',
    'm_reset_cb(1);': 'bus.m68000_reset_line(true);',
    'm_reset_cb(0);': 'bus.m68000_reset_line(false);',
}

STATE_ASSIGN_RE = re.compile(r'^m_inst_state = m_next_state \? m_next_state : m_decode_table\[m_ird\];$')
TERNARY_RE = re.compile(r'(?P<cond>[^?:()]+?)\s*\?\s*(?P<yes>[^?:]+?)\s*:\s*(?P<no>[^?:]+)')
FIELD_RE = re.compile(r'(?<![\w.])m_([A-Za-z0-9_]+)\b')
U16_FIELDS = {
    'isr', 'sr', 'new_sr', 'dbin', 'dbout', 'edb', 'irc', 'ir', 'ird', 'ftu',
    'aluo', 'alue', 'alub', 'movemr', 'irdi', 'base_ssw', 'ssw',
}
U8_FIELDS = {'dcr'}
WRAPPER_FIELDS = {
    'bcount', 'count_before_instruction_step', 'decode_table', 'icount',
}


def qualify_runtime_fields(code: str) -> str:
    def qualify(match: re.Match[str]) -> str:
        field = match.group(1)
        if field in WRAPPER_FIELDS:
            return match.group(0)
        return f'self.state.m_{field}'

    return re.sub(r'\bself\.m_([A-Za-z0-9_]+)\b', qualify, code)


def extract_functions(text: str):
    lines = text.splitlines()
    functions = []
    index = 0
    while index < len(lines):
        match = re.match(r'void m68000_device::([A-Za-z0-9_]+)\(\)(?:\s*//.*)?$', lines[index])
        if not match:
            index += 1
            continue
        name = match.group(1)
        index += 1
        assert lines[index].strip() == '{'
        index += 1
        depth = 1
        body = []
        while index < len(lines):
            line = lines[index]
            stripped = line.strip()
            depth += stripped.count('{')
            depth -= stripped.count('}')
            if depth == 0:
                index += 1
                break
            body.append(line)
            index += 1
        functions.append((name, body))
    return functions


def extract_table(text: str):
    marker = 'const m68000_device::handler m68000_device::s_handlers_if[] = {'
    start = text.index(marker) + len(marker)
    end = text.index('};', start)
    table_text = text[start:end]
    return re.findall(r'&m68000_device::([A-Za-z0-9_]+)', table_text)


def translate_decode(cpp: str) -> str:
    entries = []
    for match in re.finditer(r'\{\s*(0x[0-9a-fA-F]+),\s*(0x[0-9a-fA-F]+),\s*(\d+|S_[A-Za-z0-9_]+)\s*\}', cpp):
        entries.append((match.group(1), match.group(2), match.group(3)))
    out = []
    out.append('const PACKED_DECODE_TABLE: &[(u16, u16, u32)] = &[')
    for value, mask, state in entries:
        out.append(f'    ({value}, {mask}, {state}),')
    out.append('];')
    return '\n'.join(out)


def strip_outer_parens(s: str) -> str:
    s = s.strip()
    while s.startswith('(') and s.endswith(')'):
        depth = 0
        ok = True
        for i, ch in enumerate(s):
            if ch == '(':
                depth += 1
            elif ch == ')':
                depth -= 1
                if depth == 0 and i != len(s) - 1:
                    ok = False
                    break
        if ok:
            s = s[1:-1].strip()
        else:
            break
    return s


def translate_ternary(expr: str) -> str:
    expr = expr.strip()
    # Known generated ternaries. These patterns are intentionally narrow.
    expr = re.sub(r'\(([^()]+?)\s*\?\s*([^():]+?)\s*:\s*([^()]+?)\)',
                  lambda m: f'(if {translate_condition(m.group(1))} {{ {translate_expr(m.group(2))} }} else {{ {translate_expr(m.group(3))} }})', expr)
    expr = re.sub(r'([^()&|;+\-*/=<>]+?)\s*\?\s*([^?:;]+?)\s*:\s*([^?:;]+)',
                  lambda m: f'if {translate_condition(m.group(1))} {{ {translate_expr(m.group(2))} }} else {{ {translate_expr(m.group(3))} }}', expr)
    return expr


def translate_expr(expr: str) -> str:
    expr = expr.strip()
    expr = re.sub(r'~(?=[A-Za-z0-9_(])', '!', expr)
    expr = expr.replace('m_aob & 1 ? 0x00ff : 0xff00', 'if (m_aob & 1) != 0 { 0x00ff } else { 0xff00 }')
    expr = expr.replace('(m_aob & 1) ? 0x00ff : 0xff00', 'if (m_aob & 1) != 0 { 0x00ff } else { 0xff00 }')
    expr = expr.replace('(rx < 15 ? 1 : 2)', '(if rx < 15 { 1 } else { 2 })')
    expr = expr.replace('(ry < 15 ? 1 : 2)', '(if ry < 15 { 1 } else { 2 })')
    expr = expr.replace('(m_sr & SR_S ? SSW_S : 0)', '(if (m_sr & SR_S) != 0 { SSW_S } else { 0 })')
    expr = expr.replace('!(m_au & 0x3f) ? 0 : (m_alue & 3) == 1 ? 1 : (m_alue & 3) == 2 ? 2 : 3', 'if (m_au & 0x3f) == 0 { 0 } else if (m_alue & 3) == 1 { 1 } else if (m_alue & 3) == 2 { 2 } else { 3 }')
    expr = expr.replace('!(m_au & 0x3f) ? 0 : m_alue & 2 ? 1 : 2', 'if (m_au & 0x3f) == 0 { 0 } else if (m_alue & 2) != 0 { 1 } else { 2 }')
    expr = expr.replace('(m_isr & SR_Z) ? 2 : (m_isr & SR_N) ? 1 : 0', 'if (m_isr & SR_Z) != 0 { 2 } else if (m_isr & SR_N) != 0 { 1 } else { 0 }')
    expr = expr.replace('!(m_isr & (SR_V|SR_N))', 'u32::from((m_isr & (SR_V|SR_N)) == 0)')
    expr = expr.replace('!m_movemr', 'u32::from(m_movemr == 0)')
    expr = expr.replace('!(m_au & 0x3f)', 'u32::from((m_au & 0x3f) == 0)')
    for flag in ['SR_C', 'SR_N', 'SR_V', 'SR_Z']:
        expr = expr.replace(f'!(m_sr & {flag})', f'u32::from((m_sr & {flag}) == 0)')
    expr = expr.replace('((m_sr & (SR_N|SR_V)) == (SR_N|SR_V)) || ((m_sr & (SR_N|SR_V)) == 0)', 'u32::from(((m_sr & (SR_N|SR_V)) == (SR_N|SR_V)) || ((m_sr & (SR_N|SR_V)) == 0))')
    expr = expr.replace('((m_sr & (SR_N|SR_V)) == SR_N) || ((m_sr & (SR_N|SR_V)) == SR_V)', 'u32::from(((m_sr & (SR_N|SR_V)) == SR_N) || ((m_sr & (SR_N|SR_V)) == SR_V))')
    expr = expr.replace('((m_sr & (SR_N|SR_V|SR_Z)) == (SR_N|SR_V)) || ((m_sr & (SR_N|SR_V|SR_Z)) == 0)', 'u32::from(((m_sr & (SR_N|SR_V|SR_Z)) == (SR_N|SR_V)) || ((m_sr & (SR_N|SR_V|SR_Z)) == 0))')
    expr = expr.replace('(m_isr & (SR_Z|SR_N)) != 0', 'u32::from((m_isr & (SR_Z|SR_N)) != 0)')
    expr = expr.replace('(m_sr & (SR_C|SR_Z)) != 0', 'u32::from((m_sr & (SR_C|SR_Z)) != 0)')
    expr = expr.replace('(m_sr & (SR_C|SR_Z)) == 0', 'u32::from((m_sr & (SR_C|SR_Z)) == 0)')
    expr = expr.replace('(m_sr & SR_Z) || ((m_sr & (SR_N|SR_V)) == SR_N) || ((m_sr & (SR_N|SR_V)) == SR_V)', 'u32::from(((m_sr & SR_Z) != 0) || ((m_sr & (SR_N|SR_V)) == SR_N) || ((m_sr & (SR_N|SR_V)) == SR_V))')
    expr = expr.replace('true', 'true').replace('false', 'false')
    expr = expr.replace('m_mmu->read_program', 'self.read_program')
    expr = expr.replace('m_mmu->read_data', 'self.read_data')
    expr = expr.replace('m_mmu->read_cpu', 'self.read_cpu')
    expr = expr.replace('m_mmu->write_data', 'self.write_data')
    for name in HELPER_METHODS:
        expr = re.sub(rf'(?<![\w.]){name}\s*\(', f'self.{name}(', expr)
    expr = FIELD_RE.sub(lambda m: f'self.m_{m.group(1)}', expr)
    # Fix method calls that need bus as first argument.
    expr = expr.replace('self.read_program(', 'self.read_program(bus, ')
    expr = expr.replace('self.read_data(', 'self.read_data(bus, ')
    expr = expr.replace('self.read_cpu(', 'self.read_cpu(bus, ')
    expr = expr.replace('self.write_data(', 'self.write_data(bus, ')
    expr = expr.replace('self.m_t = (self.m_sr & SR_Z) || u32::from(((self.m_sr & (SR_N|SR_V)) == SR_N) || ((self.m_sr & (SR_N|SR_V)) == SR_V));', 'self.m_t = u32::from(((self.m_sr & SR_Z) != 0) || ((self.m_sr & (SR_N|SR_V)) == SR_N) || ((self.m_sr & (SR_N|SR_V)) == SR_V));')
    return expr


def translate_condition(cond: str) -> str:
    cond = strip_outer_parens(translate_expr(cond))
    cond = cond.replace('&&', '&&').replace('||', '||')
    if cond.startswith('!'):
        inner = strip_outer_parens(cond[1:].strip())
        if re.search(r'[<>=]|\btrue\b|\bfalse\b|\(|&&|\|\|', inner):
            return f'!({inner})'
        return f'({inner}) == 0'
    if re.search(r'[<>=]|\btrue\b|\bfalse\b|\(|&&|\|\|', cond):
        return cond
    if '&' in cond or '|' in cond or cond.startswith('self.m_'):
        return f'({cond}) != 0'
    return cond


def truncate_narrow_assignment(line: str) -> str:
    match = re.match(r'^(self\.m_([A-Za-z0-9_]+)\s*=\s*)(.*);$', line)
    if not match:
        return line
    prefix, field, value = match.groups()
    if field in U16_FIELDS:
        return f'{prefix}({value}) & 0xffff;'
    if field in U8_FIELDS:
        return f'{prefix}({value}) & 0xff;'
    return line


def translate_wrapping_assignment(line: str) -> str | None:
    match = re.match(r'self\.m_au = (.*) ([+-]) (.*);$', line)
    if not match:
        return None
    left, op, right = match.groups()
    method = 'wrapping_add' if op == '+' else 'wrapping_sub'
    return f'self.m_au = ({left}).{method}({right});'


def translate_simple_line(raw: str) -> list[str]:
    indent = raw[:len(raw) - len(raw.lstrip())].replace('\t', '    ')
    s = raw.strip()
    if not s:
        return ['']
    if s.startswith('//'):
        return [indent + s]
    if s in NOOP_LINES:
        return [indent + NOOP_LINES[s]]
    if s == 'if(!m_cmpild_instr_callback.isnull()) (m_cmpild_instr_callback)(ry, (m_dt & 0xffff0000) | m_dbin);':
        return [indent + 'self.cmpild_instr_callback(bus, ry, (self.m_dt & 0xffff0000) | self.m_dbin);']
    if s == 'if(!m_rte_instr_callback.isnull()) (m_rte_instr_callback)(1);':
        return [indent + 'self.rte_instr_callback(bus, true);']
    chained = re.match(r'm_(sr|new_sr) = m_isr = (.*);$', s)
    if chained:
        target = chained.group(1)
        value = translate_expr(chained.group(2))
        return [indent + f'self.m_isr = {value};', indent + f'self.m_{target} = self.m_isr;']
    if s == 'if(!m_tas_write_callback.isnull())':
        return [indent + '// TAS write callback is handled by write_tas_data.']
    if s.startswith('m_tas_write_callback(') or s.startswith('write_tas_data('):
        return [indent + 'self.write_tas_data(bus, self.m_aob, self.m_dbout);']
    if s == 'else':
        return [indent + '// normal TAS write path handled above']
    if s.startswith('m_mmu->write_data') and 'm_tas_write_callback' in raw:
        return []
    if re.match(r'[A-Za-z_][A-Za-z0-9_]*:$', s):
        return []
    goto = re.match(r'goto ([A-Za-z_][A-Za-z0-9_]*);$', s)
    if goto:
        return [indent + f'label = {goto.group(1).upper()};', indent + 'continue;']
    decl = re.match(r'int (rx|ry) = (.*);$', s)
    if decl:
        return [indent + f'let {decl.group(1)} = ({translate_expr(decl.group(2))}) as usize;']
    if STATE_ASSIGN_RE.match(s):
        return [indent + 'self.m_inst_state = if self.m_next_state != 0 { self.m_next_state } else { self.m_decode_table[self.m_ird as usize] };']
    # one-line if with statement
    m = re.match(r'if\((.*)\) (.*);$', s)
    if m:
        cond = translate_condition(m.group(1))
        stmt = translate_simple_line(indent + m.group(2) + ';')[-1].strip()
        return [indent + f'if {cond} {{', indent + '    ' + stmt, indent + '}']
    # if(condition) {
    m = re.match(r'if\((.*)\) \{$', s)
    if m:
        return [indent + f'if {translate_condition(m.group(1))} {{']
    # if(condition)
    m = re.match(r'if\((.*)\)$', s)
    if m:
        return [indent + f'if {translate_condition(m.group(1))}']
    # function-like write_data in normal line
    if s.startswith('m_mmu->write_data'):
        expr = translate_expr(s[:-1])
        return [indent + expr + ';']
    # assignment/call general
    expr = translate_expr(s)
    # Mutating set_* first argument.
    expr = re.sub(r'\bset_(16h|16l|8|8h|8xl|8xh)\(([^,]+),', r'set_\1(&mut \2,', expr)
    expr = expr.replace('set_16h(&mut self.m_at, self.m_at);', '{ let value = self.m_at; set_16h(&mut self.m_at, value); }')
    wrapped = translate_wrapping_assignment(expr)
    if wrapped:
        expr = wrapped
    expr = truncate_narrow_assignment(expr)
    # m_icount +=/-= remains signed.
    return [indent + expr]




def normalize_single_statement_control(lines: list[str]) -> list[str]:
    out: list[str] = []
    index = 0
    while index < len(lines):
        raw = lines[index]
        stripped = raw.strip()
        indent = raw[:len(raw) - len(raw.lstrip())]
        match_if = re.match(r'if\((.*)\)$', stripped)
        match_else_if = re.match(r'else if\((.*)\)$', stripped)
        match_close_else_if = re.match(r'} else if\((.*)\)$', stripped)
        if (stripped == '} else' or match_close_else_if) and index + 1 < len(lines):
            keyword = '} else' if stripped == '} else' else f'}} else if({match_close_else_if.group(1)})'
            out.append(indent + keyword + ' {')
            out.append(lines[index + 1])
            out.append(indent + '}')
            index += 2
            continue
        if (match_if or match_else_if or stripped == 'else') and index + 1 < len(lines):
            keyword = stripped
            if match_if:
                keyword = f'if({match_if.group(1)})'
            elif match_else_if:
                keyword = f'else if({match_else_if.group(1)})'
            out.append(indent + keyword + ' {')
            out.append(lines[index + 1])
            out.append(indent + '}')
            index += 2
            continue
        out.append(raw)
        index += 1
    return out


def translate_lines(lines: list[str]) -> list[str]:
    out: list[str] = []
    for raw in normalize_single_statement_control(lines):
        s = raw.strip()
        match_close_else_if_braced = re.match(r'} else if\((.*)\) \{$', s)
        if match_close_else_if_braced:
            indent = raw[:len(raw) - len(raw.lstrip())].replace('\t', '    ')
            out.append(indent + f'}} else if {translate_condition(match_close_else_if_braced.group(1))} {{')
            continue
        if s == '} else {':
            indent = raw[:len(raw) - len(raw.lstrip())].replace('\t', '    ')
            out.append(indent + '} else {')
            continue
        match_else_if_braced = re.match(r'else if\((.*)\) \{$', s)
        if match_else_if_braced:
            indent = raw[:len(raw) - len(raw.lstrip())].replace('\t', '    ')
            out.append(indent + f'else if {translate_condition(match_else_if_braced.group(1))} {{')
            continue
        if s == 'else {':
            indent = raw[:len(raw) - len(raw.lstrip())].replace('\t', '    ')
            out.append(indent + 'else {')
            continue
        out.extend(translate_simple_line(raw))
    return out

def has_labels(body: list[str]) -> bool:
    return any(re.match(r'\s*[A-Za-z_][A-Za-z0-9_]*:\s*$', line) for line in body)


def split_blocks(body: list[str]):
    blocks: list[tuple[str, list[str]]] = [('ENTRY', [])]
    for line in body:
        match = re.match(r'\s*([A-Za-z_][A-Za-z0-9_]*):\s*$', line)
        if match:
            blocks.append((match.group(1).upper(), []))
        else:
            blocks[-1][1].append(line)
    return blocks


def block_ends_control(lines: list[str]) -> bool:
    for line in reversed(lines):
        s = line.strip()
        if not s or s.startswith('//'):
            continue
        return s in {'return;', 'continue;'} or s.startswith('return ')
    return False


def translate_function(name: str, body: list[str]) -> str:
    out = [f'    fn {name}<B: Bus>(&mut self, bus: &mut B) {{']
    if has_labels(body):
        hoisted = []
        filtered_body = []
        for raw_line in body:
            stripped = raw_line.strip()
            decl = re.match(r'int (rx|ry) = (.*);$', stripped)
            if decl:
                hoisted.append(f'        let {decl.group(1)} = ({translate_expr(decl.group(2))}) as usize;')
            else:
                filtered_body.append(raw_line)
        out.extend(hoisted)
        blocks = split_blocks(filtered_body)
        ids = {label: i for i, (label, _) in enumerate(blocks)}
        for label, value in ids.items():
            out.append(f'        const {label}: u32 = {value};')
        out.append('        let mut label = ENTRY;')
        out.append('        loop {')
        out.append('            match label {')
        for idx, (label, lines) in enumerate(blocks):
            out.append(f'                {label} => {{')
            translated = translate_lines(lines)
            # Remove TAS normal else write after callback block: easier cleanup pass below.
            for t in translated:
                if t:
                    out.append('                    ' + t.lstrip())
            if not block_ends_control(translated):
                if idx + 1 < len(blocks):
                    out.append(f'                    label = {blocks[idx + 1][0]};')
                    out.append('                    continue;')
                else:
                    out.append('                    break;')
            out.append('                }')
        out.append('                _ => unreachable!(),')
        out.append('            }')
        out.append('        }')
    else:
        for t in translate_lines(body):
            out.append('    ' + t if t else '')
    out.append('    }')
    return '\n'.join(out)


def cleanup_tas(code: str) -> str:
    # After translating TAS callback blocks, remove the normal write_data line that follows the synthetic write_tas_data.
    code = re.sub(
        r'(self\.write_tas_data\(bus, self\.m_aob, self\.m_dbout\);\n\s*)// normal TAS write path handled above\n\s*self\.write_data\(bus, self\.m_aob & !1, self\.m_dbout, \(if .*?\}\)\);',
        r'\1', code, flags=re.S)
    code = re.sub(
        r'(self\.write_tas_data\(bus, self\.m_aob, self\.m_dbout\);\n\s*)// normal TAS write path handled above\n\s*self\.write_data\(bus, self\.m_aob & !1, self\.m_dbout, if .*?\);',
        r'\1', code, flags=re.S)
    return code


def generate(sif: str, decode: str) -> str:
    text = Path(sif).read_text()
    text = re.sub(r'\n\tif\(!m_tas_write_callback\.isnull\(\)\)\n\t\tm_tas_write_callback\(m_aob, m_dbout\);\n\telse\n\t\tm_mmu->write_data\([^;]+\);', r'\n\twrite_tas_data(m_aob, m_dbout);', text)
    functions = extract_functions(text)
    table = extract_table(text)
    out = []
    out.append('// Generated from vendored MAME m68000 tables. See README.md.')
    out.append('#[allow(clippy::all, unused_variables, unused_parens, unreachable_code, non_snake_case)]')
    out.append('impl M68000 {')
    for name, body in functions:
        out.append(translate_function(name, body))
    out.append('    fn dispatch_full<B: Bus>(&mut self, bus: &mut B, state: u32) {')
    out.append('        match state {')
    for idx, name in enumerate(table):
        out.append(f'            {idx} => self.{name}(bus),')
    out.append('            _ => self.state_illegal_if(bus),')
    out.append('        }')
    out.append('    }')
    out.append('}')
    out.append('')
    out.append(translate_decode(Path(decode).read_text()))
    return qualify_runtime_fields(cleanup_tas('\n'.join(out)))


if __name__ == '__main__':
    generated = generate(sys.argv[1], sys.argv[2])
    if len(sys.argv) > 3:
        Path(sys.argv[3]).write_text(generated)
    else:
        print(generated)
