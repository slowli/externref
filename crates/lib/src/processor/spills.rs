//! Promotion of reference spills into function locals.

use std::{collections::HashMap, mem};

use walrus::{
    FunctionId, GlobalId, InstrLocId, LocalFunction, LocalId, MemoryId, Module, ModuleLocals,
    ModuleTypes, ValType,
    ir::{self, BinaryOp, Instr, InstrSeqId, InstrSeqType, Value},
};

use super::{EXTERNREF, Error, functions::get_offset};

#[derive(Debug, Clone, Copy, PartialEq)]
enum StackValue {
    Unknown,
    Constant(i32),
    GlobalAddress(GlobalId, i64),
    Address(LocalId, i64),
    UnknownAddress(LocalId),
    Reference(ValType),
}

type Slot = (MemoryId, LocalId, i64);
type ActiveSlots = HashMap<Slot, Option<ValType>>;
type Instructions = Vec<(Instr, InstrLocId)>;

pub(super) struct ReferenceSpills<'a> {
    locals: &'a mut ModuleLocals,
    types: &'a mut ModuleTypes,
    signatures: &'a HashMap<FunctionId, (usize, Vec<ValType>)>,
    globals: &'a HashMap<GlobalId, ValType>,
    slots: HashMap<Slot, LocalId>,
    active_slots: ActiveSlots,
    exits: HashMap<InstrSeqId, Vec<ActiveSlots>>,
    loop_entries: HashMap<InstrSeqId, ActiveSlots>,
    frames: HashMap<LocalId, i64>,
    origins: HashMap<LocalId, (GlobalId, i64)>,
    escaped_addresses: Vec<StackValue>,
    references: HashMap<LocalId, LocalId>,
    copies: HashMap<(LocalId, ValType), LocalId>,
    addresses: HashMap<LocalId, StackValue>,
    function_name: Option<String>,
}

impl ReferenceSpills<'_> {
    pub fn process(
        module: &mut Module,
        functions: impl IntoIterator<Item = FunctionId>,
    ) -> Result<(), Error> {
        let signatures = module
            .funcs
            .iter()
            .map(|function| {
                let ty = module.types.get(function.ty());
                (function.id(), (ty.params().len(), ty.results().to_vec()))
            })
            .collect();
        let globals = module
            .globals
            .iter()
            .map(|global| (global.id(), global.ty))
            .collect();
        for function in functions {
            let function = module.funcs.get_mut(function);
            let mut spills = ReferenceSpills {
                locals: &mut module.locals,
                types: &mut module.types,
                signatures: &signatures,
                globals: &globals,
                slots: HashMap::new(),
                active_slots: HashMap::new(),
                exits: HashMap::new(),
                loop_entries: HashMap::new(),
                frames: HashMap::new(),
                origins: HashMap::new(),
                escaped_addresses: vec![],
                references: HashMap::new(),
                copies: HashMap::new(),
                addresses: HashMap::new(),
                function_name: function.name.clone(),
            };
            let function = function.kind.unwrap_local_mut();
            spills.sequence(function, function.entry_block(), vec![])?;
        }
        Ok(())
    }

    fn error(&self, location: InstrLocId) -> Error {
        Error::UnsupportedReferenceSpill {
            function_name: self.function_name.clone(),
            code_offset: get_offset(location),
        }
    }

    fn slot(memory: MemoryId, offset: u64, address: StackValue) -> Option<Slot> {
        if let StackValue::Address(local, displacement) = address {
            Some((
                memory,
                local,
                displacement.checked_add(i64::try_from(offset).ok()?)?,
            ))
        } else {
            None
        }
    }

    fn overlaps(left: Slot, width: u32, right: Slot) -> bool {
        left.0 == right.0
            && left.1 == right.1
            && left.2 < right.2 + 4
            && right.2 < left.2 + i64::from(width)
    }

    fn merge_slots(&mut self, other: &ActiveSlots) {
        for (&slot, ty) in &mut self.active_slots {
            if other.get(&slot) != Some(ty) {
                *ty = None;
            }
        }
        for slot in other.keys() {
            self.active_slots.entry(*slot).or_insert(None);
        }
    }

    fn aliases_slot(value: StackValue, slot: Slot) -> bool {
        match value {
            StackValue::Address(local, displacement) => {
                slot.1 == local && slot.2 <= displacement && displacement < slot.2 + 4
            }
            StackValue::UnknownAddress(local) => slot.1 == local,
            _ => false,
        }
    }

    fn aliases_spill(&self, value: StackValue) -> bool {
        self.active_slots
            .keys()
            .any(|&slot| Self::aliases_slot(value, slot))
    }

    fn assign(
        &mut self,
        local: &mut LocalId,
        value: StackValue,
        location: InstrLocId,
    ) -> Result<StackValue, Error> {
        if self.frames.contains_key(local) && self.active_slots.keys().any(|slot| slot.1 == *local)
        {
            return Err(self.error(location));
        }
        self.frames.remove(local);
        self.origins.remove(local);
        self.references.remove(local);
        self.addresses.remove(local);
        match value {
            StackValue::Reference(ty) if self.locals.get(*local).ty() == ValType::I32 => {
                let replacement = *self
                    .copies
                    .entry((*local, ty))
                    .or_insert_with(|| self.locals.add(ty));
                self.references.insert(*local, replacement);
                *local = replacement;
            }
            StackValue::GlobalAddress(global, displacement) => {
                self.origins.insert(*local, (global, displacement));
                let address = StackValue::Address(*local, 0);
                self.addresses.insert(*local, address);
                return Ok(address);
            }
            StackValue::Address(_, _) | StackValue::UnknownAddress(_) => {
                self.addresses.insert(*local, value);
            }
            _ => {}
        }
        Ok(value)
    }

    fn binary_value(
        &self,
        op: BinaryOp,
        left: StackValue,
        right: StackValue,
        location: InstrLocId,
    ) -> Result<StackValue, Error> {
        if matches!(left, StackValue::Reference(_)) || matches!(right, StackValue::Reference(_)) {
            return Err(self.error(location));
        }
        Ok(match (op, left, right) {
            (
                BinaryOp::I32Add,
                StackValue::GlobalAddress(base, offset),
                StackValue::Constant(value),
            )
            | (
                BinaryOp::I32Add,
                StackValue::Constant(value),
                StackValue::GlobalAddress(base, offset),
            ) => StackValue::GlobalAddress(base, offset + i64::from(value)),
            (
                BinaryOp::I32Sub,
                StackValue::GlobalAddress(base, offset),
                StackValue::Constant(value),
            ) => StackValue::GlobalAddress(base, offset - i64::from(value)),
            (BinaryOp::I32Add, StackValue::Address(base, offset), StackValue::Constant(value))
            | (BinaryOp::I32Add, StackValue::Constant(value), StackValue::Address(base, offset)) => {
                StackValue::Address(base, offset + i64::from(value))
            }
            (BinaryOp::I32Sub, StackValue::Address(base, offset), StackValue::Constant(value)) => {
                StackValue::Address(base, offset - i64::from(value))
            }
            (
                BinaryOp::I32Add | BinaryOp::I32Sub,
                StackValue::Address(base, _) | StackValue::UnknownAddress(base),
                _,
            )
            | (
                BinaryOp::I32Add,
                _,
                StackValue::Address(base, _) | StackValue::UnknownAddress(base),
            ) => StackValue::UnknownAddress(base),
            _ => StackValue::Unknown,
        })
    }

    fn call(
        &mut self,
        call: &ir::Call,
        stack: &mut Vec<StackValue>,
        location: InstrLocId,
    ) -> Result<(), Error> {
        let (params, results) = &self.signatures[&call.func];
        let start = stack.len().saturating_sub(*params);
        if stack[start..]
            .iter()
            .any(|&value| self.aliases_spill(value))
        {
            return Err(self.error(location));
        }
        self.escaped_addresses
            .extend(stack[start..].iter().copied().filter(|value| {
                matches!(
                    value,
                    StackValue::Address(_, _) | StackValue::UnknownAddress(_)
                )
            }));
        stack.truncate(start);
        stack.extend(results.iter().map(|&ty| match ty {
            ValType::Ref(_) => StackValue::Reference(ty),
            _ => StackValue::Unknown,
        }));
        Ok(())
    }

    fn simple_instruction(
        &mut self,
        instruction: &mut Instr,
        stack: &mut Vec<StackValue>,
        location: InstrLocId,
    ) -> Result<bool, Error> {
        match instruction {
            Instr::LocalGet(get) => {
                if let Some(replacement) = self.references.get(&get.local) {
                    get.local = *replacement;
                }
                stack.push(match self.locals.get(get.local).ty() {
                    ty @ ValType::Ref(_) => StackValue::Reference(ty),
                    ValType::I32 => self
                        .addresses
                        .get(&get.local)
                        .copied()
                        .unwrap_or(StackValue::Address(get.local, 0)),
                    _ => StackValue::Unknown,
                });
            }
            Instr::Const(constant) => stack.push(match constant.value {
                Value::I32(value) => StackValue::Constant(value),
                _ => StackValue::Unknown,
            }),
            Instr::LocalSet(set) => {
                self.assign(
                    &mut set.local,
                    stack.pop().unwrap_or(StackValue::Unknown),
                    location,
                )?;
            }
            Instr::LocalTee(tee) => {
                let value = self.assign(
                    &mut tee.local,
                    stack.last().copied().unwrap_or(StackValue::Unknown),
                    location,
                )?;
                if let Some(top) = stack.last_mut() {
                    *top = value;
                }
            }
            Instr::GlobalGet(global) => {
                stack.push(if self.globals[&global.global] == ValType::I32 {
                    StackValue::GlobalAddress(global.global, 0)
                } else {
                    StackValue::Unknown
                });
            }
            Instr::GlobalSet(global) => {
                if let Some(value @ StackValue::Address(base, displacement)) = stack.pop() {
                    if let Some(&(origin, adjustment)) = self.origins.get(&base) {
                        if origin == global.global && adjustment < 0 && displacement == 0 {
                            self.frames.insert(base, -adjustment);
                        } else if origin != global.global {
                            self.escaped_addresses.push(value);
                        }
                    }
                }
            }
            Instr::Drop(_) => {
                stack.pop();
            }
            Instr::Binop(binary) => {
                let right = stack.pop().unwrap_or(StackValue::Unknown);
                let left = stack.pop().unwrap_or(StackValue::Unknown);
                stack.push(self.binary_value(binary.op, left, right, location)?);
            }
            Instr::Unop(_) => {
                if matches!(stack.pop(), Some(StackValue::Reference(_))) {
                    return Err(self.error(location));
                }
                stack.push(StackValue::Unknown);
            }
            Instr::Call(call) => self.call(call, stack, location)?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn store(
        &mut self,
        store: &ir::Store,
        stack: &mut Vec<StackValue>,
        rewritten: &mut Instructions,
        location: InstrLocId,
    ) -> Result<bool, Error> {
        let value = stack.pop().unwrap_or(StackValue::Unknown);
        let address = stack.pop().unwrap_or(StackValue::Unknown);
        if let StackValue::Reference(ty) = value {
            if !matches!(store.kind, ir::StoreKind::I32 { atomic: false }) {
                return Err(self.error(location));
            }
            let key = Self::slot(store.memory, store.arg.offset, address)
                .ok_or_else(|| self.error(location))?;
            if self
                .frames
                .get(&key.1)
                .is_none_or(|&size| key.2 < 0 || key.2 + 4 > size)
                || self
                    .active_slots
                    .keys()
                    .any(|&other| other != key && Self::overlaps(key, 4, other))
                || self
                    .escaped_addresses
                    .iter()
                    .any(|&value| Self::aliases_slot(value, key))
            {
                return Err(self.error(location));
            }
            let local = *self
                .slots
                .entry(key)
                .or_insert_with(|| self.locals.add(EXTERNREF));
            self.active_slots.insert(key, Some(ty));
            rewritten.push((Instr::LocalSet(ir::LocalSet { local }), location));
            rewritten.push((
                Instr::Const(ir::Const {
                    value: Value::I32(0),
                }),
                InstrLocId::default(),
            ));
            return Ok(true);
        }
        if matches!(
            value,
            StackValue::Address(_, _) | StackValue::UnknownAddress(_)
        ) {
            self.escaped_addresses.push(value);
        }
        if let Some(key) = Self::slot(store.memory, store.arg.offset, address) {
            let width = store.kind.width();
            if self.active_slots.keys().any(|&other| {
                Self::overlaps(key, width, other)
                    && (key.2 > other.2 || key.2 + i64::from(width) < other.2 + 4)
            }) {
                return Err(self.error(location));
            }
            self.active_slots
                .retain(|&other, _| !Self::overlaps(key, width, other));
        } else if self.aliases_spill(address) {
            return Err(self.error(location));
        }
        Ok(false)
    }

    fn load(
        &self,
        load: &ir::Load,
        stack: &mut Vec<StackValue>,
        location: InstrLocId,
    ) -> Result<Option<(LocalId, ValType)>, Error> {
        let address = stack.pop().unwrap_or(StackValue::Unknown);
        let key = Self::slot(load.memory, load.arg.offset, address);
        let slot = key.and_then(|key| {
            self.active_slots
                .iter()
                .find(|&(&other, _)| Self::overlaps(key, load.kind.width(), other))
                .map(|(&other, &ty)| (key, other, ty))
        });
        if let Some((key, other, ty)) = slot {
            let Some(ty) = ty else {
                return Err(self.error(location));
            };
            if key != other || !matches!(load.kind, ir::LoadKind::I32 { atomic: false }) {
                return Err(self.error(location));
            }
            stack.push(StackValue::Reference(ty));
            return Ok(Some((self.slots[&key], ty)));
        }
        if key.is_none() && self.aliases_spill(address) {
            return Err(self.error(location));
        }
        stack.push(StackValue::Unknown);
        Ok(None)
    }

    fn load_slot(rewritten: &mut Instructions, local: LocalId, ty: ValType) {
        rewritten.push((Instr::Drop(ir::Drop {}), InstrLocId::default()));
        rewritten.push((
            Instr::LocalGet(ir::LocalGet { local }),
            InstrLocId::default(),
        ));
        if let ValType::Ref(reference) = ty {
            if !reference.nullable {
                rewritten.push((
                    Instr::RefAsNonNull(ir::RefAsNonNull {}),
                    InstrLocId::default(),
                ));
            }
        }
    }

    fn branch(
        &mut self,
        function: &mut LocalFunction,
        branch: &ir::IfElse,
        stack: &mut Vec<StackValue>,
        location: InstrLocId,
    ) -> Result<(), Error> {
        stack.pop();
        let references = self.references.clone();
        let addresses = self.addresses.clone();
        let incoming_slots = self.active_slots.clone();
        let consequent = self.sequence(function, branch.consequent, vec![])?;
        let consequent_references = self.references.clone();
        let consequent_addresses = self.addresses.clone();
        let consequent_slots = self.active_slots.clone();
        self.references = references;
        self.addresses = addresses;
        self.active_slots = incoming_slots;
        let alternative = self.sequence(function, branch.alternative, vec![])?;
        if self.references != consequent_references {
            return Err(self.error(location));
        }
        self.addresses
            .retain(|local, address| consequent_addresses.get(local) == Some(address));
        self.merge_slots(&consequent_slots);
        stack.extend(
            consequent
                .into_iter()
                .zip(alternative)
                .map(|(left, right)| match (left, right) {
                    (StackValue::Reference(left), StackValue::Reference(right))
                        if left == right =>
                    {
                        StackValue::Reference(left)
                    }
                    _ => StackValue::Unknown,
                }),
        );
        Ok(())
    }

    fn sequence(
        &mut self,
        function: &mut LocalFunction,
        sequence: InstrSeqId,
        mut stack: Vec<StackValue>,
    ) -> Result<Vec<StackValue>, Error> {
        let sequence_type = function.block(sequence).ty;
        let results = match sequence_type {
            InstrSeqType::Simple(result) => result.into_iter().collect::<Vec<_>>(),
            InstrSeqType::MultiValue(ty) => self.types.get(ty).results().to_vec(),
        };
        let instructions = mem::take(&mut function.block_mut(sequence).instrs);
        let mut rewritten = Vec::with_capacity(instructions.len());
        for (mut instruction, mut location) in instructions {
            if self.simple_instruction(&mut instruction, &mut stack, location)? {
                rewritten.push((instruction, location));
                continue;
            }
            match &mut instruction {
                Instr::Store(store) => {
                    if self.store(store, &mut stack, &mut rewritten, location)? {
                        location = InstrLocId::default();
                    }
                }
                Instr::Load(load) => {
                    let slot = self.load(load, &mut stack, location)?;
                    rewritten.push((instruction, location));
                    if let Some((local, ty)) = slot {
                        Self::load_slot(&mut rewritten, local, ty);
                    }
                    continue;
                }
                Instr::Block(block) => {
                    stack.extend(self.sequence(function, block.seq, vec![])?);
                }
                Instr::Loop(block) => {
                    let incoming = self.active_slots.clone();
                    self.loop_entries.insert(block.seq, incoming.clone());
                    stack.extend(self.sequence(function, block.seq, vec![])?);
                    self.merge_slots(&incoming);
                }
                Instr::IfElse(branch) => {
                    self.branch(function, branch, &mut stack, location)?;
                }
                Instr::BrIf(branch) => {
                    stack.pop();
                    self.exits
                        .entry(branch.block)
                        .or_default()
                        .push(self.active_slots.clone());
                }
                Instr::Br(branch) => {
                    self.exits
                        .entry(branch.block)
                        .or_default()
                        .push(self.active_slots.clone());
                    stack.clear();
                }
                Instr::BrTable(branch) => {
                    for target in branch.blocks.iter().chain([&branch.default]) {
                        self.exits
                            .entry(*target)
                            .or_default()
                            .push(self.active_slots.clone());
                    }
                    stack.clear();
                }
                Instr::Return(_) | Instr::Unreachable(_) => {
                    stack.clear();
                }
                _ => {
                    if stack
                        .iter()
                        .any(|value| matches!(value, StackValue::Reference(_)))
                    {
                        return Err(self.error(location));
                    }
                    stack.clear();
                }
            }
            rewritten.push((instruction, location));
        }
        function.block_mut(sequence).instrs = rewritten;
        for exit in self.exits.remove(&sequence).unwrap_or_default() {
            if self
                .loop_entries
                .get(&sequence)
                .is_some_and(|incoming| incoming != &exit)
            {
                return Err(self.error(InstrLocId::default()));
            }
            self.merge_slots(&exit);
        }
        self.loop_entries.remove(&sequence);
        let mut values = vec![StackValue::Unknown; results.len()];
        for (value, actual) in values.iter_mut().rev().zip(stack.into_iter().rev()) {
            *value = actual;
        }
        if values.iter().zip(&results).any(|(value, expected)| matches!(value, StackValue::Reference(actual) if actual != expected)) {
            return Err(self.error(InstrLocId::default()));
        }
        Ok(values)
    }
}
