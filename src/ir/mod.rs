mod dtype;
mod interp;
mod write_ir;

use core::convert::TryFrom;
use core::fmt;
use core::ops::Deref;
use lang_c::ast;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

pub use dtype::{Dtype, DtypeError, HasDtype};
pub use interp::{interp, Value};

#[derive(Debug, PartialEq)]
pub struct TranslationUnit {
    pub decls: HashMap<String, Declaration>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Declaration {
    Variable {
        dtype: Dtype,
        initializer: Option<Constant>,
    },
    Function {
        signature: FunctionSignature,
        definition: Option<FunctionDefinition>,
    },
}

impl TryFrom<Dtype> for Declaration {
    type Error = DtypeError;

    /// Create an appropriate declaration according to `dtype`.
    ///
    /// # Example
    ///
    /// If `int g = 0;` is declared, `dtype` is
    /// `ir::Dtype::Int{ width:32, is_signed:true, is_const:false }`.
    /// In this case, `ir::Declaration::Variable{ dtype, initializer: Some(Constant::I32(1)) }`
    /// is generated.
    ///
    /// Conversely, if `int foo();` is declared, `dtype` is
    /// `ir::Dtype::Function{ret: Scalar(Int), params: []}`.
    /// Thus, in this case, `ir::Declaration::Function` is generated.
    fn try_from(dtype: Dtype) -> Result<Self, Self::Error> {
        match &dtype {
            Dtype::Unit { .. } => Err(DtypeError::Misc {
                message: "A variable of type `void` cannot be declared".to_string(),
            }),
            Dtype::Int { .. } | Dtype::Float { .. } | Dtype::Pointer { .. } => {
                Ok(Declaration::Variable {
                    dtype,
                    initializer: None,
                })
            }
            Dtype::Function { .. } => Ok(Declaration::Function {
                signature: FunctionSignature::new(dtype),
                definition: None,
            }),
        }
    }
}

impl Declaration {
    pub fn get_variable(&self) -> Option<(&Dtype, &Option<Constant>)> {
        if let Self::Variable { dtype, initializer } = self {
            Some((dtype, initializer))
        } else {
            None
        }
    }

    pub fn get_function(&self) -> Option<(&FunctionSignature, &Option<FunctionDefinition>)> {
        if let Self::Function {
            signature,
            definition,
        } = self
        {
            Some((signature, definition))
        } else {
            None
        }
    }

    pub fn get_function_mut(
        &mut self,
    ) -> Option<(&mut FunctionSignature, &mut Option<FunctionDefinition>)> {
        if let Self::Function {
            signature,
            definition,
        } = self
        {
            Some((signature, definition))
        } else {
            None
        }
    }

    /// Check if type is conflicting for pre-declared one
    ///
    /// In case of `Variable`, need to check if the two types are exactly the same.
    /// On the other hand, in the case of `Function`, outermost `const` of return type and
    /// parameters one is not an issue of concern.
    pub fn is_compatible(&self, other: &Declaration) -> bool {
        match (self, other) {
            (Self::Variable { dtype, .. }, Self::Variable { dtype: other, .. }) => dtype == other,
            (
                Self::Function { signature, .. },
                Self::Function {
                    signature: other, ..
                },
            ) => signature.dtype().is_compatible(&other.dtype()),
            _ => false,
        }
    }
}

impl HasDtype for Declaration {
    fn dtype(&self) -> Dtype {
        match self {
            Self::Variable { dtype, .. } => dtype.clone(),
            Self::Function { signature, .. } => signature.dtype(),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct FunctionDefinition {
    /// Memory allocations for local variables.  The allocation is performed at the beginning of a
    /// function invocation.
    pub allocations: Vec<Dtype>,

    /// Basic blocks.
    pub blocks: HashMap<BlockId, Block>,

    /// The initial block id.
    pub bid_init: BlockId,
}

#[derive(Debug, PartialEq, Clone)]
pub struct FunctionSignature {
    pub ret: Dtype,
    pub params: Vec<Dtype>,
}

impl FunctionSignature {
    pub fn new(dtype: Dtype) -> Self {
        let (ret, params) = dtype
            .get_function_inner()
            .expect("function signature's dtype must be function type");
        Self {
            ret: ret.clone(),
            params: params.clone(),
        }
    }
}

impl HasDtype for FunctionSignature {
    fn dtype(&self) -> Dtype {
        Dtype::function(self.ret.clone(), self.params.clone())
    }
}

#[derive(Debug, Eq, Clone)]
pub enum RegisterId {
    /// Registers holding pointers to local allocations.
    ///
    /// # Fields
    ///
    /// - `name`: only for debugging purposes.
    /// - `id`: local allocation id.
    Local { name: String, id: usize },

    /// Registers holding block arguments.
    ///
    /// # Fields
    ///
    /// - `bid`: When it is the initial block id, then it holds a function argument; otherwise, it
    ///   holds a phinode value.
    /// - `aid`: the argument index.
    Arg { bid: BlockId, aid: usize },

    /// Registers holding the results of instructions.
    ///
    /// # Fields
    ///
    /// - `bid`: the instruction's block id.
    /// - `iid`: the instruction's id in the block.
    Temp { bid: BlockId, iid: usize },
}

impl RegisterId {
    pub fn local(name: String, id: usize) -> Self {
        Self::Local { name, id }
    }

    pub fn arg(bid: BlockId, aid: usize) -> Self {
        Self::Arg { bid, aid }
    }

    pub fn temp(bid: BlockId, iid: usize) -> Self {
        Self::Temp { bid, iid }
    }
}

impl fmt::Display for RegisterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local { name, id } => write!(f, "%(local:{}:{})", name, id),
            Self::Arg { bid, aid } => write!(f, "%(arg:{}:{})", bid, aid),
            Self::Temp { bid, iid } => write!(f, "%({}:{})", bid, iid),
        }
    }
}

impl PartialEq<RegisterId> for RegisterId {
    fn eq(&self, other: &RegisterId) -> bool {
        match (self, other) {
            (Self::Local { id, .. }, Self::Local { id: other_id, .. }) => id == other_id,
            (
                Self::Arg { bid, aid },
                Self::Arg {
                    bid: other_bid,
                    aid: other_aid,
                },
            ) => bid == other_bid && aid == other_aid,
            (
                Self::Temp { bid, iid },
                Self::Temp {
                    bid: other_bid,
                    iid: other_iid,
                },
            ) => bid == other_bid && iid == other_iid,
            _ => false,
        }
    }
}

impl Hash for RegisterId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Local { id, .. } => id.hash(state),
            Self::Arg { bid, aid } => {
                // TODO: needs to distinguish arg/temp?
                bid.hash(state);
                aid.hash(state);
            }
            Self::Temp { bid, iid } => {
                bid.hash(state);
                iid.hash(state);
            }
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Constant {
    Unit,
    Int {
        value: u128,
        width: usize,
        is_signed: bool,
    },
    Float {
        /// `value` may be `f32`, but it is possible to consider it as `f64`.
        ///
        /// * Casting from an f32 to an f64 is perfect and lossless (f32 -> f64)
        /// * Casting from an f64 to an f32 will produce the closest possible value (f64 -> f32)
        /// https://doc.rust-lang.org/stable/reference/expressions/operator-expr.html#type-cast-expressions
        value: f64,
        width: usize,
    },
    GlobalVariable {
        name: String,
        dtype: Dtype,
    },
}

impl TryFrom<&ast::Constant> for Constant {
    type Error = ();

    fn try_from(constant: &ast::Constant) -> Result<Self, Self::Error> {
        match constant {
            ast::Constant::Integer(integer) => {
                let is_signed = !integer.suffix.unsigned;

                let dtype = match integer.suffix.size {
                    ast::IntegerSize::Int => Dtype::INT,
                    ast::IntegerSize::Long => Dtype::LONG,
                    ast::IntegerSize::LongLong => Dtype::LONGLONG,
                }
                .set_signed(is_signed);

                let value = if is_signed {
                    integer.number.parse::<i128>().unwrap() as u128
                } else {
                    integer.number.parse::<u128>().unwrap()
                };

                Ok(Self::int(value, dtype))
            }
            ast::Constant::Float(float) => {
                let (dtype, value) = match float.suffix.format {
                    ast::FloatFormat::Float => {
                        // Casting from an f32 to an f64 is perfect and lossless (f32 -> f64)
                        // https://doc.rust-lang.org/stable/reference/expressions/operator-expr.html#type-cast-expressions
                        (Dtype::FLOAT, float.number.parse::<f32>().unwrap() as f64)
                    }
                    ast::FloatFormat::Double => {
                        (Dtype::DOUBLE, float.number.parse::<f64>().unwrap())
                    }
                    ast::FloatFormat::LongDouble => {
                        panic!("`FloatFormat::LongDouble` is_unsupported")
                    }
                    ast::FloatFormat::TS18661Format(_) => {
                        panic!("`FloatFormat::TS18661Format` is_unsupported")
                    }
                };

                Ok(Self::float(value, dtype))
            }
            ast::Constant::Character(character) => {
                let dtype = Dtype::CHAR;
                let value = character.parse::<char>().unwrap() as u128;

                Ok(Self::int(value, dtype))
            }
        }
    }
}

impl TryFrom<&ast::Expression> for Constant {
    type Error = ();

    fn try_from(expr: &ast::Expression) -> Result<Self, Self::Error> {
        if let ast::Expression::Constant(constant) = expr {
            Self::try_from(&constant.node)
        } else {
            Err(())
        }
    }
}

impl TryFrom<&ast::Initializer> for Constant {
    type Error = ();

    fn try_from(initializer: &ast::Initializer) -> Result<Self, Self::Error> {
        if let ast::Initializer::Expression(expr) = &initializer {
            Self::try_from(&expr.node)
        } else {
            Err(())
        }
    }
}

impl Constant {
    #[inline]
    pub fn is_integer_constant(&self) -> bool {
        if let Self::Int { .. } = self {
            true
        } else {
            false
        }
    }

    pub fn unit() -> Self {
        Constant::Unit
    }

    #[inline]
    pub fn int(value: u128, dtype: Dtype) -> Self {
        let width = dtype.get_int_width().expect("`dtype` must be `Dtype::Int`");
        let is_signed = dtype.is_int_signed();

        Constant::Int {
            value,
            width,
            is_signed,
        }
    }

    #[inline]
    pub fn float(value: f64, dtype: Dtype) -> Self {
        let width = dtype
            .get_float_width()
            .expect("`dtype` must be `Dtype::Float`");

        Constant::Float { value, width }
    }

    #[inline]
    pub fn global_variable(name: String, dtype: Dtype) -> Self {
        Self::GlobalVariable { name, dtype }
    }
}

impl fmt::Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => write!(f, "unit"),
            Self::Int { value, .. } => write!(f, "{}", value),
            Self::Float { value, .. } => write!(f, "{}", value),
            Self::GlobalVariable { name, .. } => write!(f, "%{}", name),
        }
    }
}

impl HasDtype for Constant {
    fn dtype(&self) -> Dtype {
        match self {
            Self::Unit => Dtype::unit(),
            Self::Int {
                width, is_signed, ..
            } => Dtype::int(*width).set_signed(*is_signed),
            Self::Float { width, .. } => Dtype::float(*width),
            Self::GlobalVariable { dtype, .. } => Dtype::pointer(dtype.clone()),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Operand {
    Constant(Constant),
    Register { rid: RegisterId, dtype: Dtype },
}

impl Operand {
    pub fn constant(value: Constant) -> Self {
        Self::Constant(value)
    }

    pub fn register(rid: RegisterId, dtype: Dtype) -> Self {
        Self::Register { rid, dtype }
    }

    pub fn get_constant(&self) -> Option<&Constant> {
        if let Self::Constant(constant) = self {
            Some(constant)
        } else {
            None
        }
    }

    pub fn get_register(&self) -> Option<(&RegisterId, &Dtype)> {
        if let Self::Register { rid, dtype } = self {
            Some((rid, dtype))
        } else {
            None
        }
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Constant(value) => write!(f, "{}", value),
            Self::Register { rid, .. } => write!(f, "{}", rid),
        }
    }
}

impl HasDtype for Operand {
    fn dtype(&self) -> Dtype {
        match self {
            Self::Constant(value) => value.dtype(),
            Self::Register { dtype, .. } => dtype.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Instruction {
    // TODO: the variants of Instruction will be added in the future
    BinOp {
        op: ast::BinaryOperator,
        lhs: Operand,
        rhs: Operand,
        dtype: Dtype,
    },
    UnaryOp {
        op: ast::UnaryOperator,
        operand: Operand,
        dtype: Dtype,
    },
    Store {
        ptr: Operand,
        value: Operand,
    },
    Load {
        ptr: Operand,
    },
    Call {
        callee: Operand,
        args: Vec<Operand>,
        return_type: Dtype,
    },
    TypeCast {
        value: Operand,
        target_dtype: Dtype,
    },
}

impl HasDtype for Instruction {
    fn dtype(&self) -> Dtype {
        match self {
            Self::BinOp { dtype, .. } => dtype.clone(),
            Self::UnaryOp { dtype, .. } => dtype.clone(),
            Self::Store { .. } => Dtype::unit(),
            Self::Load { ptr } => ptr
                .dtype()
                .get_pointer_inner()
                .expect("Load instruction must have pointer value as operand")
                .deref()
                .clone()
                .set_const(false),
            Self::Call { return_type, .. } => return_type.clone(),
            Self::TypeCast { target_dtype, .. } => target_dtype.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct BlockId(pub usize);

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b{}", self.0)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct JumpArg {
    pub bid: BlockId,
    pub args: Vec<Operand>,
}

impl JumpArg {
    pub fn new(bid: BlockId, args: Vec<Operand>) -> Self {
        Self { bid, args }
    }
}

impl fmt::Display for JumpArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({:?})", self.bid, self.args)
    }
}

// TODO
#[derive(Debug, PartialEq, Clone)]
pub enum BlockExit {
    Jump {
        arg: JumpArg,
    },
    ConditionalJump {
        condition: Operand,
        arg_then: JumpArg,
        arg_else: JumpArg,
    },
    Switch {
        value: Operand,
        default: JumpArg,
        cases: Vec<(Constant, JumpArg)>,
    },
    Return {
        value: Operand,
    },
    Unreachable,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Block {
    pub phinodes: Vec<Dtype>,
    pub instructions: Vec<Instruction>,
    pub exit: BlockExit,
}