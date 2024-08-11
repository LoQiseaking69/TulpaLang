use inkwell::context::Context;
use crate::parser::{ASTNode};

pub struct LLVMCompiler<'ctx> {
    context: &'ctx Context,
}

impl<'ctx> LLVMCompiler<'ctx> {
    pub fn new(context: &'ctx Context) -> Self {
        LLVMCompiler { context }
    }

    pub fn compile(&self, ast: ASTNode) {
        let module = self.context.create_module("tulpa");
        let builder = self.context.create_builder();

        // Create a function prototype for example
        let fn_type = self.context.i32_type().fn_type(&[], false);
        let function = module.add_function("main", fn_type, None);
        let basic_block = self.context.append_basic_block(function, "entry");
        builder.position_at_end(basic_block);

        // The actual compilation logic will involve translating the AST to LLVM IR
        // Here, you would recursively traverse the AST, creating corresponding LLVM instructions.

        builder.build_return(Some(&self.context.i32_type().const_int(0, false)));

        // Optionally print the LLVM IR to verify
        module.print_to_stderr();
    }
}
