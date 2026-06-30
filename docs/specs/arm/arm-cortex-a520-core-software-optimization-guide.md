# Arm Cortex-A520 Core Software Optimization Guide

Source title: Arm® Cortex®-A520 Core Software Optimization Guide.
Document: `PJDOC-1505342170-671342`; metadata version: `0001`; metadata version label: `r0p1`; revision: `1.2`.
Cover: core revision `r0p1`; issue `1.2`.
Published: `2024-03-22`; updated: `2024-12-12`; product quality: `EAC`.
Source PDF: `docs/arm_cortex_a520_core_software_optimization_guide.pdf`; SHA-256: `49ce6c7f1e6a3c7b060f36a86ebee733a3a6e724afb74dfff5ca26b993c7b049`.

## 1 Introduction

### 1.1 Product revision status

The rmpn identifier indicates the revision status of the product described in this book, for
example, r1p2, where:

rm
Identifies the major revision of the product, for example, r1.

```text
 pn      Identifies the minor revision or modification status of the product, for
         example, p2.
```

### 1.2 Intended audience

This document is for system designers, system integrators, and programmers who are
designing or programming a System-on-Chip (SoC) that uses an Arm core.

### 1.3 Conventions

The following subsections describe conventions used in Arm documents.

#### 1.3.1 Glossary

The Arm Glossary is a list of terms used in Arm documentation, together with definitions for
those terms. The Arm Glossary does not contain terms that are industry standard unless the
Arm meaning differs from the generally accepted meaning.

See the Arm Glossary for more information: https://developer.arm.com/glossary.

Terms and abbreviations
This document uses the following terms and abbreviations.

```text
  Convention                           Use
  ALU                                  Arithmetic and Logical Unit
  ASIMD                                Advanced SIMD
  FP                                   Floating-point
  GPR                                  General Purpose Register
  SQRT                                 Square Root
  SVE                                  Scalable Vector instruction Extension (SVE or SVE2)
  VPR                                  Vector Processing Register; FP/ASIMD/SVE registers
  Convention                           Use
  VPU                                  Vector Processing Unit
```

#### 1.3.2 Typographical conventions

```text
  Convention           Use
  italic               Citations.
  bold                 Interface elements, such as menu names.
                       Signal names.
                       Terms in descriptive lists, where appropriate.
  monospace            Text that you can enter at the keyboard, such as commands, file and
                       program names, and source code.
  monospace            Language keywords when used outside example code.
  bold
  monospace            A permitted abbreviation for a command or option. You can enter the
  underline            underlined text instead of the full command or option name.
  <and>                Encloses replaceable terms for assembler syntax where they appear in
                       code or code fragments.
                       For example:
                       MRC p15, 0, <Rd>, <CRn>, <CRm>, <Opcode_2>

  SMALL                Terms that have specific technical meanings as defined in the Arm®
  CAPITALS             Glossary. For example, IMPLEMENTATION DEFINED, IMPLEMENTATION
                       SPECIFIC, UNKNOWN, and UNPREDICTABLE.

                       Recommendations. Not following these recommendations might lead
                       to system failure or damage.

                       Requirements for the system. Not following these requirements might
                       result in system failure or damage.

                       Requirements for the system. Not following these requirements will
                       result in system failure or damage.
```

An important piece of information that needs your attention.

A useful tip that might make it easier, better, or faster to perform a
task.

```text
  Convention           Use
                       A reminder of something important that relates to the information you
                       are reading.
```

### 1.4 Additional reading

This document contains information that is specific to this product. See the following
documents for other relevant information:

Table 1-1 Arm publications

```text
  Document name                                 Document ID                            Licensee only Y/N
  Arm® Architecture Reference                   DDI 0487                               N
  Manual for A-profile architecture
  profile
  Arm® Cortex-A520 Core Technical               102517                                 N
  Reference Manual
  Arm® Cortex-A520 Core                         102518                                 Y
  Configuration and Integration
  Manual

Arm tests its PDFs only in Adobe Acrobat and Acrobat Reader. Arm cannot guarantee the
quality of its documents when used with any other PDF reader.
```

Adobe PDF reader products can be downloaded at http://www.adobe.com.

## 2 Overview

Cortex-A520 Core is a high-efficiency, low-power product that implements the Arm®v9.2-A
architecture. The Arm®v9.2-A architecture extends the architecture defined in the Arm®v8‑A
architectures up to Arm®v8.7-A.

The key features of Cortex-A520 Core are:

- Implementation of the Arm®v9.2-A A64 instruction set

- AArch64 Execution state at all Exception levels, EL0 to EL3

- Separate L1 data and instruction side memory systems with a Memory Management
Unit (MMU)

- In-order pipeline with direct and indirect branch prediction

- Generic Interrupt Controller (GIC) CPU interface to connect to an external interrupt
distributor

- Generic Timer interface that supports a 64-bit count input from an external system
counter

- Implementation of the Reliability, Availability, and Serviceability (RAS) Extension

- 128-bit Scalable Vector Extension (SVE) and SVE2 SIMD instruction set, offering
Advanced SIMD (ASIMD) and floating-point (FP) architecture support

- Support for the optional Cryptographic Extension, which is licensed separately

- Activity Monitoring Unit (AMU)

- Dual/Single Core configuration option: Cortex-A520 cores can be grouped into dual-
core complexes or instantiated as single-core complexes. Dual-core complexes share
the L2 cache and VPU, while single-core complexes have a dedicated L2 cache and VPU.

**Figure 1 highlights the VPU pipelines shared between Cortex-A520 cores in a complex.**

- Configurable vector datapath size: The size of the vector datapaths can be 2x64 or
2x128-bit. The selected option applies to all cores in the complex. Figure 1 highlights
the VPU pipelines that are only instantiated for a 2x128-bit configuration.

This document describes the elements of Cortex-A520 Core micro-architecture that influence
the software performance so that software and compilers can be optimized accordingly.

### 2.1 Pipeline overview

```text
       IF0    IF1    IF2        DE0 DE1 DE2           ISS         EX1 EX2 EX3 WR           RET

             Fetch                  Decode                                 ALU0

                                                                           ALU1

                                                                          Branch

                                                                           DIV

                                                                        Load/Store

                                                                           Load

                                                                           MAC

                                                                           PAC
                                                      Issue

                                                                   V0    V1    V2     V3    V4       V5   RC

                                                                                 Crypto0

                                                                                  VALU0

                                                                                 VMAC0

                                                                                                                        Shared VPU
                                                                                     VMC

                                                                                                                                     VPU
                                                                                 Crypto1
                                                                                                          VPU 128-bit
                                                                                  VALU1

                                                                                 VMAC1
```

**Figure 1 Cortex-A520 Core pipeline**

The execution pipelines support different types of operations, as shown in the following table.

```text
  Pipeline                 Instructions
  ALU0,                    Arithmetic and logic
  ALU1
  Branch                   Branch
  Pipeline           Instructions
  Crypto0            Cryptography
                     Supports 1x128-bit operation.
                     This pipeline is shared for dual core configuration.
                     Present only for implementations configured with Cryptographic
                     Extensions enabled.
  Crypto1            Cryptography
                     Supports 1x128-bit operation.
                     This pipeline is shared for dual core configuration.
                     Present only for implementations configured with Cryptographic
                     Extensions enabled and a Vector datapath size of 2x128-bit.
  DIV                Integer scalar division (iterative)
  Load/Store         Load and store
  Load               Load
  MAC                Multiply accumulate
  PAC                Pointer Authentication
  VALU0              Addition, logic and shift for ASIMD, FP, Neon, and SVE
                     Supports 2x64-bit or 1x128-bit operations.
                     This pipeline is shared for dual core configuration.
  VALU1              Addition, logic and shift for ASIMD, FP, Neon, and SVE
                     Supports 2x64-bit or 1x128-bit operations.
                     This pipeline is shared for dual core configuration.
                     Present only for implementations configured with a Vector datapath
                     size of 2x128-bit.
  VMAC0              Multiply accumulate for ASIMD, FP, Neon, and SVE
                     Supports 2x64-bit or 1x128-bit operations.
                     This pipeline is shared for dual core configurations.
  VMAC1              Multiply accumulate for ASIMD, FP, Neon, and SVE
                     Supports 2x64-bit or 1x128-bit operations.
                     This pipeline is shared for dual core configurations.
                     Present only for implementations configured with a Vector datapath
                     size of 2x128-bit configurations.
  VMC                Cryptography and iterative multi cycle instruction (e.g. bit permutation,
                     division, and square root)
                     Supports 2x64-bit or 1x128-bit operations.
                     This pipeline is shared for dual core configurations.
```

## 3 Instruction characteristics

### 3.1 Instruction tables

This chapter describes high-level performance characteristics for most Armv9-A instructions.
A series of tables summarize the effective execution latency and throughput (instruction
bandwidth per cycle), pipelines utilized, and special behaviors associated with each group of
instructions. Utilized pipelines correspond to the execution pipelines described in chapter 2.

In the tables below:

- Exec Latency is the minimum latency seen by an operation dependent on an instruction
in the described group.

- Load Latency is the minimum latency seen by an operation dependent on the load. It is
assumed the memory access hits in the L1 Data Cache.

- Execution Throughput is maximum throughput (in instructions per cycle) of the specified
instruction group that can be achieved in the entirety of Cortex-A520 Core
microarchitecture.

The Vector datapath size may affect the operation of ASIMD, FP, Neon, and SVE instructions.
In such cases the Exec Latency and Execution Throughput will be defined with two value, “A,B”. A
is for a 2x128-bit configuration or a non-Q or scalar form of a 2x64-bit configuration. B is for a
2x64-bit configuration.

### 3.2 Branch Instructions

Table 3-1 AArch64 Branch instructions

```text
  Instruction Group               AArch64                Exec                Execution             Utilized
                                  Instruction            Latency             Throughput            Pipeline
  Branch, immed                   B                                  -                     1       Branch
  Branch, register                BR, RET                            -                     1       Branch
  Branch and link,                BL                                1                      1       Branch
  immed
  Branch and link,                BLR                               1                      1       Branch
  register
  Compare and                     CBZ, CBNZ,                         -                     1       Branch
  branch                          TBZ, TBNZ
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Branch, immed                 B                                  -                     1       Branch
  Branch, register              BX                                 -                     1       Branch
  Branch and link,              BL, BLX                           1                      1       Branch
  immed
  Branch and link,              BLX                               1                      1       Branch
  register
  Compare and                   CBZ, CBNZ                          -                     1       Branch
  branch
```

### 3.3 Arithmetic and logical instructions

Table 3-2 AArch64 Arithmetic and logical instructions

```text
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Arithmetic, basic             ADD, ADC,                         1                      2       ALU
                                SUB, SBC

  Arithmetic, basic,            ADDS,                             1                      2       ALU
  flagset [1]                   SUBS
                                ADCS, SBCS                        1                      1
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Arithmetic, extend           ADD{S},                         1[1]                     2       ALU
  and shift                    SUB{S}

  Conditional                  CCMN,                             1                      1       ALU
  compare                      CCMP
  Conditional select           CSEL,                             1                      2       ALU
                               CSINC,
                               CSINV,
                               CSNEG
  Logical, basic               AND{S},                           1                      2       ALU
                               BIC{S}, EOR,
                               ORR
  Logical, shift               AND{S},                           1                      2       ALU
                               BIC{S},
                               EON, EOR,
                               ORN, ORR
```

Notes:

1. Latency=2 when the dependency is on Rm.

### 3.4 Divide and multiply instructions

Integer divides are performed using an iterative algorithm and block any subsequent divide
operations until complete. Early termination is possible, depending upon the data values.

Table 3-3 AArch64 Divide and multiply instructions1

```text
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Divide, W-form               SDIV2, UDIV                     12                   1/12        DIV
  Divide, X-form               SDIV, UDIV                      20                   1/20        DIV
  Multiply                     MADD,                             3                      1       MAC
  accumulate, W-form           MSUB
                               MUL                               3                      1
  Multiply                     MADD,                             4                   1/2        MAC
  accumulate, X-form           MSUB, MUL
  Multiply accumulate          SMADDL,                           2                      1       MAC
  long                         SMSUBL,
                               UMADDL,
                               UMSUBL
  Multiply high                SMULH,                            6                   1/4        MAC
                               UMULH
```

Notes:

1.   There is a dedicated forwarding path in the accumulate portion of the unit that allows
the result of one MAC operation to be used as the accumulate operand of a following
MAC operation with no interlock. Thanks to this, a typical sequence of multiply-
accumulate instructions can issue one every 2 cycles). Accumulator forwarding is not
supported for consumers of 64 bit multiply high operations.

2. Latency and throughput numbers given for SDIV and UDIV are the worst-case values.
Early termination is possible, depending upon the data values (for example, degenerate
cases such as divide by zero). Integer divides are performed using an iterative algorithm
and block any subsequent divide operations until complete. The number of cycles
needed to execute these instructions can be calculated using the formula [N + bits/4]
(N=3 for UDIV, N=4 for SDIV, i.e. signed division takes one more cycle than unsigned
division).

### 3.5 Pointer authentication instructions

Table 3-4 AArch64 Pointer authentication instructions

```text
  Instruction Group              AArch64                Exec                Execution             Utilized
                                 Instruction            Latency             Throughput            Pipeline
  Authenticate data              AUTDA,                             -                     1       PAC
  address                        AUTDB,
                                 AUTDZA,
                                 AUTDZB
  Authenticate                   AUTIA,                            5                      1       PAC
  instruction address            AUTIB,
                                 AUTIA1716,
                                 AUTIB1716,
                                 AUTIASP,
                                 AUTIBSP,
                                 AUTIAZ,
                                 AUTIBZ,
                                 AUTIZA,
                                 AUTIZB
  Branch and link,               BLRAA,                            1                      1       Branch,
  register, with                 BLRAAZ,                                                          PAC
  pointer                        BLRAB,
  authentication                 BLRABZ
  Branch, register,              BRAA,                              -                     1       Branch,
  with pointer                   BRAAZ,                                                           PAC
  authentication                 BRAB,
                                 BRABZ
  Branch, return, with           RETA, RETB                         -                     1       Branch
  pointer
  authentication
  Compute pointer                PACDA,                            5                      1       PAC
  authentication code            PACDB,
  for data address               PACDZA,
                                 PACDZB
  Compute pointer                PACGA                             5                      1       PAC
  authentication code,
  using generic key
  Compute pointer                PACIA,                            5                      1       PAC
  authentication code            PACIB,
  for instruction                PACIZA,
  address                        PACIZB
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
                                PACIA171,
                                PACIB1716,
                                PACIAZ,
                                PACIASP,
                                PACIBSP,
                                PACIBZ
  Load register, with           LDRAA,                            2                      2       PAC
  pointer                       LDRAB
  authentication,
  offset
  Load register, with           LDRAA,                            2                      1       PAC
  pointer                       LDRAB
  authentication, pre-
  indexed
  Strip pointer                 XPACD,                            5                   1/5        PAC
  authentication code           XPACI,
                                XPACLRI
```

### 3.6 Miscellaneous data-processing instructions

Table 3-5 AArch64 miscellaneous data-processing instructions

```text
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Address generation            ADR, ADRP                         1                      2       ALU
  Bitfield extract              EXTR                            2[1]                     2       ALU
  Bitfield move, basic          SBFM,                           2[2]                     2       ALU
                                SBFIZ,
                                SBFX,
                                SXTH,
                                SXTW,
                                UBFM,
                                UBFIZ,
                                UBFX,
                                UXTH
  Bitfield move, insert         BFM                               2                      2       ALU
  Convert floating-             AXFLAG,                            -                  1/2        ALU
  point condition flags         XAFLAG
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Flag manipulation            SETF8,                            2                   1/2        ALU
  instructions                 SETF16
                               RMIF,                             1                      1

                               CFINV                                                 1/2
  Count leading                CLS, CLZ                          1                      2       ALU
  Move immed                   MOVN,                             1                      2       ALU
                               MOVK,
                               MOVZ
  Reverse bits/bytes           REV,                              1                      2       ALU
                               REV16,
                               REV32
                               RBIT                              2                      2
  Variable shift               ASRV, LSLV,                       1                      2       ALU
                               LSRV, RORV
```

Notes:

1. Latency=1 for ROR (immediate) alias of EXTR

2. Latency=1 for LSL (immediate), LSR (immediate) and UXTB aliases of UBFM

Latency=1 for SXTB and ASR (immediate) aliases of SBFM

### 3.7 Load instructions

The latencies shown in Table 3-6 assume the memory access hits in the Level 1 Data Cache.

Base register updates are done in parallel to the operation.

Table 3-6 AArch64 Load instructions

```text
  Instruction Group            AArch64                Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Load register, literal       LDR,                              2                      2       Load/Store,
                               LDRSW,                                                           Load
                               PRFM
  Instruction Group            AArch64                Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Load register,               LDUR,                             2                      2       Load/Store,
  unscaled immed               LDURB,                                                           Load
                               LDURH,
                               LDURSB,
                               LDURSH,
                               LDURSW,
                               PRFUM
  Load register,               LDR, LDRB,                        2                      2       Load/Store,
  immed post-index             LDRH,                                                            Load
                               LDRSB,
                               LDRSH,
                               LDRSW
  Load register,               LDR, LDRB,                        2                      2       Load/Store,
  immed pre-index              LDRH,                                                            Load
                               LDRSB,
                               LDRSH,
                               LDRSW
  Load register,               LDTR,                             2                      2       Load/Store,
  immed unprivileged           LDTRB,                                                           Load
                               LDTRH,
                               LDTRSB,
                               LDTRSH,
                               LDTRSW
  Load register,               LDR, LDRB,                        2                      2       Load/Store,
  unsigned immed               LDRH,                                                            Load
                               LDRSB,
                               LDRSH,
                               LDRSW,
                               PRFM
  Load register,               LDR, LDRB,                        2                      2       Load/Store,
  register offset, basic       LDRH,                                                            Load
                               LDRSB,
                               LDRSH,
                               LDRSW,
                               PRFM
  Load register,               LDR,                              2                      2       Load/Store,
  register offset, scale       LDRSW,                                                           Load
  by 4/8                       PRFM
  Load register,               LDRH,                             2                      2       Load/Store,
  register offset, scale       LDRSH                                                            Load
  by 2
  Instruction Group             AArch64                Load                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Load register,                LDR, LDRB,                        2                      2       Load/Store,
  register offset,              LDRH,                                                            Load
  extend                        LDRSB,
                                LDRSH,
                                LDRSW,
                                PRFM
  Load register,                LDR,                              2                      2       Load/Store,
  register offset,              LDRSW,                                                           Load
  extend, scale by 4/8          PRFM
  Load register,                LDRH,                             2                      2       Load/Store,
  register offset,              LDRSH                                                            Load
  extend, scale by 2
  Load pair, signed             LDP, LDNP                         2                      2       Load/Store,
  immed offset,                                                                                  Load
  normal, W-form
  Load pair, signed             LDP, LDNP                         2                      2       Load/Store,
  immed offset,                                                                                  Load
  normal, X-form
  Load pair, signed             LDPSW                             2                      2       Load/Store,
  immed offset,                                                                                  Load
  signed words
  Load pair, immed              LDP                               2                      1       Load/Store,
  post-index or                                                                                  Load
  immed pre-index,
  normal, W-form
  Load pair, immed              LDP                               2                      1       Load/Store,
  post-index or                                                                                  Load
  immed pre-index,
  normal, X-form
  Load pair, immed              LDPSW                             2                      1       Load/Store,
  post-index, signed                                                                             Load
  words
```

### 3.8 Store instructions

Base register updates are done in parallel to the operation.
Table 3-7 AArch64 Store instructions

```text
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Store register,               STUR,                              -                     1       Load/Store
  unscaled immed                STURB,
                                STURH
  Store register,               STR, STRB,                         -                     1       Load/Store
  immed post-index              STRH
  Store register,               STR, STRB,                         -                     1       Load/Store
  immed pre-index               STRH
  Store register,               STTR,                              -                     1       Load/Store
  immed unprivileged            STTRB,
                                STTRH
  Store register,               STR, STRB,                         -                     1       Load/Store
  unsigned immed                STRH
  Store register,               STR, STRB,                         -                     1       Load/Store
  register offset, basic        STRH
  Store register,               STR                                -                     1       Load/Store
  register offset,
  scaled by 4/8
  Store register,               STRH                               -                     1       Load/Store
  register offset,
  scaled by 2
  Store register,               STR, STRB,                         -                     1       Load/Store
  register offset,              STRH
  extend
  Store register,               STR                                -                     1       Load/Store
  register offset,
  extend, scale by 4/8
  Store register,               STRH                               -                     1       Load/Store
  register offset,
  extend, scale by 1
  Store pair, immed             STP, STNP                          -                     1       Load/Store
  offset
  Store pair, immed             STP                                -                     1       Load/Store
  post-index
  Store pair, immed             STP                                -                     1       Load/Store
  pre-index
```

### 3.9 Tag data processing

Table 3-8 AArch64 Tag data processing instructions

```text
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Arithmetic,                   ADDG,                             2                      2       ALU
  immediate to logical          SUBG
  address tag
  Insert Random Tags            IRG                               3                   1/3        ALU
  Insert Tag Mask               GMI                               2                      2       ALU
  Subtract Pointer              SUBP                              2                      2       ALU
  Subtract Pointer,             SUBPS                             2                      2       ALU
  flagset
```

### 3.10 Tag load instructions

The latencies shown assume the memory access hits in the Level 1 Data Cache.

Table 3-9 AArch64 Tag load instructions

```text
  Instruction Group             AArch64                Load                Execution             Utilized
                                Instructions           Latency             Throughput            Pipeline
  Load allocation tag           LDG                               2                      2       Load/Store,
                                                                                                 Load
  Load multiple                 LDGM                              2                   1/4        Load/Store,
  allocation tags                                                                                Load
```

### 3.11 Tag store instructions

Base register updates are done in parallel to the operation.

Table 3-10 AArch64 Tag store instructions

```text
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Store allocation tags         STG                                -                     1       Load/Store
  to one or two
  granules, post-index          ST2G                                                  1/2
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Store allocation tags         STG                                -                     1       Load/Store
  to one or two
  granules, pre-index           ST2G                                                  1/2

  Store allocation tags         STG                                -                     1       Load/Store
  to one or two
  granules, signed              ST2G                                                  1/2
  offset
  Store allocation tag          STZG                               -                     1       Load/Store
  to one or two
  granules, zeroing,            STZ2G                                                 1/2
  post-index
  Store Allocation Tag          STZG                               -                     1       Load/Store
  to one or two
  granules, zeroing,            STZ2G                                                 1/2
  pre-index
  Store allocation tag          STZG                               -                     1       Load/Store
  to two granules,
  zeroing, signed               STZ2G                                                 1/2
  offset
  Store allocation tag          STGP                               -                     1       Load/Store
  and reg pair to
  memory, post-Index
  Store allocation tag          STGP                               -                     1       Load/Store
  and reg pair to
  memory, pre-Index
  Store allocation tag          STGP                               -                     1       Load/Store
  and reg pair to
  memory, signed
  offset
  Store multiple                STGM                               -                     1       Load/Store
  allocation tags
  Store multiple                STZGM                              -                     1       Load/Store
  allocation tags,
  zeroing
```

### 3.12 FP scalar data processing instructions

Table 3-11 AArch64 FP data processing instructions

```text
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  FP absolute value            FABS, FABD                        4                      2       VALU
  FP arithmetic                FADD,                             4                      2       VALU
                               FSUB
  FP compare                   FCCMP{E},                         5                   1/5        VALU
                               FCMP{E}                           1                      1
  FP divide, H-form 1          FDIV                              8                   2/5        VMC
  FP divide, S-form 1          FDIV                            13                   2/10        VMC
  FP divide, D-form 1          FDIV                            22                   2/19        VMC
  FP min/max                   FMIN,                             4                      2       VALU
                               FMINNM,
                               FMAX,
                               FMAXNM
  FP multiply                  FMUL,                             4                      2       VMAC
                               FNMUL
  FP multiply                  FMADD,                            4                      2       VMAC
  accumulate                   FMSUB,
                               FNMADD,
                               FNMSUB
  FP negate                    FNEG                              4                      2       VALU
  FP round to integral         FRINTA,                           4                      2       VALU
                               FRINTI,
                               FRINTM,
                               FRINTN,
                               FRINTP,
                               FRINTX,
                               FRINTZ,
                               FRINT32X,
                               FRINT64X,
                               FRINT32Z,
                               FRINT64Z
  FP select                    FCSEL                             3                      1       VALU
  FP square root, H-           FSQRT                           11                    2/5        VMC
  form
  FP square root, S-           FSQRT                           14                    2/9        VMC
  form
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  FP square root, D-           FSQRT                           25                   2/19        VMC
  form
```

Notes:

1. Floating-point division operations may finish early if the divisor is a power of two
(normal with a zero trailing significand).

### 3.13 FP scalar miscellaneous instructions

Table 3-12 AArch64 FP miscellaneous instructions

```text
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  FP convert, from             SCVTF,                            4                      2       VALU
  gen to vec reg               UCVTF
  FP convert, from             FCVTAS,                           4                      1       VALU
  vec to gen reg               FCVTAU,
                               FCVTMS,
                               FCVTMU,
                               FCVTNS,
                               FCVTNU,
                               FCVTPS,
                               FCVTPU,
                               FCVTZS,
                               FCVTZU
  FP convert,                  FJCVTZS                           4                      1       VALU
  Javascript from vec
  to gen reg
  FP convert, from             FCVT,                             4                      2       VALU
  vec to vec reg               FCVTXN
  FP move, immed               FMOV                              3                      2       VALU
  FP move, register            FMOV                              3                      1       VALU
  FP transfer, from            FMOV                              3                      2       VALU
  gen to vec reg
  FP transfer, from            FMOV                              3                      1       VALU
  vec to gen reg
```

### 3.14 FP scalar load instructions

The latencies shown assume the memory access hits in the Level 1 Data Cache.

Base register updates are done in parallel to the operation.

Table 3-13 AArch64 FP load instructions

```text
  Instruction Group             AArch64                Load                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Load vector reg,              LDR                               3                      2       Load/Store,
  literal, S/D/Q forms                                                                           Load
  Load vector reg,              LDUR                              3                      2       Load/Store,
  unscaled immed                                                                                 Load
  Load vector reg,              LDR                               3                      2       Load/Store,
  immed post-index                                                                               Load
  Load vector reg,              LDR                               3                      2       Load/Store,
  immed pre-index                                                                                Load
  Load vector reg,              LDR                               3                      2       Load/Store,
  unsigned immed                                                                                 Load
  Load vector reg,              LDR                               3                      2       Load/Store,
  register offset, basic                                                                         Load
  Load vector reg,              LDR                               3                      2       Load/Store,
  register offset,                                                                               Load
  scale, S/D-form
  Load vector reg,              LDR                               3                      2       Load/Store,
  register offset,                                                                               Load
  scale, H/Q-form
  Load vector reg,              LDR                               3                      2       Load/Store,
  register offset,                                                                               Load
  extend
  Load vector reg,              LDR                               3                      2       Load/Store,
  register offset,                                                                               Load
  extend, scale, S/D-
  form
  Load vector reg,              LDR                               3                      2       Load/Store,
  register offset,                                                                               Load
  extend, scale, H/Q-
  form
  Load vector pair,             LDP, LDNP                         3                      1       Load/Store,
  immed offset, S/D-                                                                             Load
  form
  Instruction Group            AArch64                Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Load vector pair,            LDP, LDNP                         3                      1       Load/Store,
  immed offset, Q-                                                                              Load
  form
  Load vector pair,            LDP                               3                      1       Load/Store,
  immed post-index,                                                                             Load
  S/D-form
  Load vector pair,            LDP                               3                      1       Load/Store,
  immed post-index,                                                                             Load
  Q-form
  Load vector pair,            LDP                               3                      1       Load/Store,
  immed pre-index,                                                                              Load
  S/D-form
  Load vector pair,            LDP                               3                      1       Load/Store,
  immed pre-index,                                                                              Load
  Q-form
```

### 3.15 FP scalar store instructions

Base register updates are done in parallel to the operation.

Table 3-14 AArch64 FP Store instructions

```text
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instructions           Latency             Throughput            Pipeline
  Store vector reg,            STUR                               -                     1       Load/Store
  unscaled immed,
  B/H/S/D-form
  Store vector reg,            STUR                               -                     1       Load/Store
  unscaled immed, Q-
  form
  Store vector reg,            STR                                -                     1       Load/Store
  immed post-index,
  B/H/S/D-form
  Store vector reg,            STR                                -                     1       Load/Store
  immed post-index,
  Q-form
  Store vector reg,            STR                                -                     1       Load/Store
  immed pre-index,
  B/H/S/D-form
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instructions           Latency             Throughput            Pipeline
  Store vector reg,            STR                                -                     1       Load/Store
  immed pre-index,
  Q-form
  Store vector reg,            STR                                -                     1       Load/Store
  unsigned immed,
  B/H/S/D-form
  Store vector reg,            STR                                -                     1       Load/Store
  unsigned immed, Q-
  form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  basic, B/H/S/D-
  form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  basic, Q-form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  scale, H-form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  scale, S/D-form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  scale, Q-form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  extend, B/H/S/D-
  form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  extend, Q-form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  extend, scale, H-
  form
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  extend, scale, S/D-
  form
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instructions           Latency             Throughput            Pipeline
  Store vector reg,            STR                                -                     1       Load/Store
  register offset,
  extend, scale, Q-
  form
  Store vector pair,           STP, STNP                          -                     1       Load/Store
  immed offset, S-
  form
  Store vector pair,           STP, STNP                          -                     1       Load/Store
  immed offset, D-
  form
  Store vector pair,           STP, STNP                         2                   1/2        Load/Store
  immed offset, Q-
  form
  Store vector pair,           STP                                -                     1       Load/Store
  immed post-index,
  S-form
  Store vector pair,           STP                                -                     1       Load/Store
  immed post-index,
  D-form
  Store vector pair,           STP                               2                   1/2        Load/Store
  immed post-index,
  Q-form
  Store vector pair,           STP                                -                     1       Load/Store
  immed pre-index, S-
  form
  Store vector pair,           STP                                -                     1       Load/Store
  immed pre-index, D-
  form
  Store vector pair,           STP                               2                   1/2        Load/Store
  immed pre-index,
  Q-form
```

### 3.16 ASIMD Integer instructions

Table 3-15 AArch64 ASIMD Integer instructions

```text
  Instruction Group            AArch64                  Exec               Execution                Utilized
                               Instruction              Latency            Throughput               Pipeline
  ASIMD absolute diff          SABD, UABD                             3                   2,1       VALU
  Instruction Group            AArch64                  Exec               Execution                Utilized
                               Instruction              Latency            Throughput               Pipeline
  ASIMD absolute diff          SABA, UABA                          6                2/7,1/7         VALU
  accum
  ASIMD absolute diff          SABAL(2),                           6                1/2,1/4         VALU
  accum long                   UABAL(2)
  ASIMD absolute diff          SABDL(2),                           3                      2,1       VALU
  long                         UABDL(2)
  ASIMD arith, basic           ABS, ADD,                           3                      2,1       VALU
                               NEG,
                               SHADD,
                               SHSUB, SUB,
                               UHADD,
                               UHSUB,
  ASIMD arith, basic,          SADDL(2),                           3                      2,1       VALU
  long, saturate               SADDW(2),
                               SSUBL(2),
                               SSUBW(2),
                               UADDL(2),
                               UADDW(2),
                               USUBL(2),
                               USUBW(2)
  ASIMD arith,                 ADDHN(2),                           4                      2,1       VALU
  complex                      RSUBHN(2),
                               SQABS,
                               SQADD,
                               SQNEG,
                               SQSUB,
                               SUBHN(2),
                               SUQADD,
                               UQADD,
                               UQSUB,
                               USQADD
                               RADDHN(2)                           8                2/5,1/5

                               SRHADD,                             3                      2,1
                               URHADD

  ASIMD arith, pair-           ADDP,                               3                      2,1       VALU
  wise                         SADDLP,
                               UADDLP
  Instruction Group            AArch64                  Exec               Execution                Utilized
                               Instruction              Latency            Throughput               Pipeline
  ASIMD arith,                 ADDV,                               4                          1     VALU
  reduce, 4H/4S                SADDLV,
                               UADDLV
  ASIMD arith,                 ADDV                                3                          1     VALU
  reduce
  ASIMD arith,                 SADDLV,                             4                          1     VALU
  reduce                       UADDLV
  ASIMD compare                CMEQ,                               3                      2,1       VALU
                               CMGE,
                               CMGT,
                               CMHI,
                               CMHS,
                               CMLE, CMLT
  ASIMD compare                CMTST                               4                      2,1       VALU
  test
  ASIMD dot product            SDOT, UDOT                          4                      2,1       VMAC
  ASIMD dot product            SUDOT,                              4                      2,1       VMAC
  using signed and             USDOT
  unsigned integers
  ASIMD logical                AND, BIC,                           3                      2,1       VALU
                               EOR, MOV,
                               MVN, NOT,
                               ORN, ORR
  ASIMD matrix                 SMMLA,                              4                      2,1       VALU
  multiply-accumulate          UMMLA,
                               USMMLA
  ASIMD max/min,               SMAX,                               3                      2,1       VALU
  basic and pair-wise          SMAXP,
                               SMIN,
                               SMINP,
                               UMAX,
                               UMAXP,
                               UMIN,
                               UMINP
  ASIMD max/min,               SMAXV,                              4                          1     VALU
  reduce, 4H/4S                SMINV,
                               UMAXV,
                               UMINV
  Instruction Group            AArch64                  Exec               Execution                Utilized
                               Instruction              Latency            Throughput               Pipeline
  ASIMD max/min,               SMAXV,                              4                          1     VALU
  reduce, 8B/8H                SMINV,
                               UMAXV,
                               UMINV
  ASIMD max/min,               SMAXV,                              4                          1     VALU
  reduce, 16B                  SMINV,
                               UMAXV,
                               UMINV
  ASIMD multiply               MUL,                                4                      2,1       VMAC
                               SQDMULH,
                               SQRDMULH
  ASIMD multiply               MLA, MLS                            4                      2,1       VMAC
  accumulate
  ASIMD multiply               SQRDMLAH,                           4                          1     VMAC
  accumulate high, D-          SQRDMLSH
  form
  ASIMD multiply               SQRDMLAH,                           4                          1     VMAC
  accumulate high, Q-          SQRDMLSH
  form
  ASIMD multiply               SMLAL(2),                           4                      2,1       VMAC
  accumulate long              SMLSL(2),
                               UMLAL(2),
                               UMLSL(2)
  ASIMD multiply               SQDMLAL(2),                         4                      2,1       VMAC
  accumulate                   SQDMLSL(2)
  saturating long
  ASIMD                        PMUL,                               4                      2,1       VALU
  multiply/multiply            PMULL(2)
  long (8x8)
  polynomial, D-form
  ASIMD                        PMUL,                               4                      2,1       VALU
  multiply/multiply            PMULL(2)
  long (8x8)
  polynomial, Q-form
  ASIMD multiply               SMULL(2),                           4                      2,1       VMAC
  long                         UMULL(2),
                               SQDMULL(2)
  ASIMD pairwise               SADALP,                             6                2/5,1/5         VALU
  add and accumulate           UADALP
  long
  Instruction Group            AArch64                  Exec               Execution                Utilized
                               Instruction              Latency            Throughput               Pipeline
  ASIMD shift                  SRSRA,                              8                2/5,1/5         VALU
  accumulate                   URSRA
                               SSRA, USRA                          3                      2,1
  ASIMD shift by               SHL, SHLL(2),                       3                      2,1       VALU
  immed, basic                 SSHLL(2),
                               SSHR,
                               SXTL(2),
                               USHLL(2),
                               USHR,
                               UXTL(2)
  ASIMD shift by               SHRN(2),                            4                      2,1       VALU
  immed, basic
  ASIMD shift by               SLI, SRI                            3                      2,1       VALU
  immed and insert,
  basic
  ASIMD shift by               RSHRN(2),                           4                      2,1       VALU
  immed, complex               SQRSHRN(2),
                               SQRSHRUN(
                               2), SQSHL{U},
                               SQSHRN(2),
                               SQSHRUN(2),
                               UQRSHRN(2)
                               , UQSHL,
                               UQSHRN(2),
  ASIMD shift by               SSHL, USHL,                         3                      2,1       VALU
  register, basic              SRSHL,
                               SRSHR,
                               URSHL,
                               URSHR
  ASIMD shift by               SQRSHL,                             4                      2,1       VALUE
  register, complex            SQSHL,
                               UQRSHL,
                               UQSHL
```

### 3.17 ASIMD FP data processing instructions

Table 3-16 AArch64 ASIMD Floating-point instructions

```text
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD FP absolute            FABS, FABD                        4                    2,1       VALU
  value/difference
  ASIMD FP arith,              FABD,                             4                    2,1       VALU
  normal                       FADD,
                               FSUB,
                               FADDP
  ASIMD FP compare             FACGE,                            3                    2,1       VALU
                               FACGT,
                               FCMEQ,
                               FCMGE,
                               FCMGT,
                               FCMLE,
                               FCMLT
  ASIMD FP complex             FCADD                             4                    2,1       VMAC
  add
  ASIMD FP complex             FCMLA                             4                    2,1       VMAC
  multiply add
  ASIMD FP convert,            FCVTL(2)                          4                    2,1       VALU
  long (F16 to F32)
  ASIMD FP convert,            FCVTL(2)                          4                    2,1       VALU
  long (F32 to F64)
  ASIMD FP convert,            FCVTN(2)                          4                    2,1       VALU
  narrow (F32 to F16)
  ASIMD FP convert,            FCVTN(2),                         4                    2,1       VALU
  narrow (F64 to F32)          FCVTXN(2)
  ASIMD FP convert,            FCVTAS,                           4                    2,1       VALUE
  other, D-form F32            FCVTAU,
  and Q-form F64               FCVTMS,
                               FCVTMU,
                               FCVTNS,
                               FCVTNU,
                               FCVTPS,
                               FCVTPU,
                               FCVTZS,
                               FCVTZU,
                               SCVTF,
                               UCVTF
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD FP convert,            FCVTAS,                           4                    2,1       VALU
  other, D-form F16            FCVTAU,
  and Q-form F32               FCVTMS,
                               FCVTMU,
                               FCVTNS,
                               FCVTNU,
                               FCVTPS,
                               FCVTPU,
                               FCVTZS,
                               FCVTZU,
                               SCVTF,
                               UCVTF
  ASIMD FP convert,            FCVTAS,                           4                    2,1       VALU
  other, Q-form F16            VCVTAU,
                               FCVTMS,
                               FCVTMU,
                               FCVTNS,
                               FCVTNU,
                               FCVTPS,
                               FCVTPU,
                               FCVTZS,
                               FCVTZU,
                               SCVTF,
                               UCVTF
  ASIMD FP divide,             FDIV                              8                   2/5        VMC
  D-form, F16
  ASIMD FP divide,             FDIV                            13                   2/10        VMC
  D-form, F32 1
  ASIMD FP divide,             FDIV                              8                   1/5        VMC
  Q-form, F16 1
  ASIMD FP divide,             FDIV                            13                   1/10        VMC
  Q-form, F32 1
  ASIMD FP divide,             FDIV                            22                   1/19        VALU
  Q-form, F64
  ASIMD FP max/min,            FMAX,                             4                    2,1       VALU
  normal                       FMAXNM,
                               FMIN,
                               FMINNM
  ASIMD FP max/min,            FMAXP,                            4                    2,1       VALU
  pairwise                     FMAXNMP,
                               FMINP,
                               FMINNMP
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD FP max/min,            FMAXV,                            4                      1       VALU
  reduce                       FMAXNMV,
                               FMINV,
                               FMINNMV
  ASIMD FP max/min,            FMAXV,                            4                      1       VALU
  reduce, Q-form F16           FMAXNMV,
                               FMINV,
                               FMINNMV
  ASIMD FP multiply            FMUL,                             4                    2,1       VMAC
                               FMULX
  ASIMD FP multiply            FMLA,                             4                    2,1       VMAC
  accumulate                   FMLS
  ASIMD FP multiply            FMLAL(2),                         4                    2,1       VMAC
  accumulate long              FMLSL(2)
  ASIMD FP negate              FNEG                              4                    2,1       VALU
  ASIMD FP round,              FRINTA,                           4                    2,1       VALU
  D-form F32 and Q-            FRINTI,
  form F64                     FRINTM,
                               FRINTN,
                               FRINTP,
                               FRINTX,
                               FRINTZ,
                               FRINT32X,
                               FRINT64X,
                               FRINT32Z,
                               FRINT64Z
  ASIMD FP round,              FRINTA,                           4                    2,1       VALU
  D-form F16 and Q-            FRINTI,
  form F32                     FRINTM,
                               FRINTN,
                               FRINTP,
                               FRINTX,
                               FRINTZ,
                               FRINT32X,
                               FRINT64X,
                               FRINT32Z,
                               FRINT64Z
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD FP round,              FRINTA,                           4                    2,1       VALU
  Q-form F16                   FRINTI,
                               FRINTM,
                               FRINTN,
                               FRINTP,
                               FRINTX,
                               FRINTZ,
                               FRINT32X,
                               FRINT64X,
                               FRINT32Z,
                               FRINT64Z
  ASIMD FP square              FSQRT                             8                   2/5        VMC
  root, D-form, F16
  ASIMD FP square              FSQRT                           12                    2/9        VMC
  root, D-form, F32
  ASIMD FP square              FSQRT                             8                   1/5        VMC
  root, Q-form, F16
  ASIMD FP square              FSQRT                           12                    1/9        VMC
  root, Q-form, F32
  ASIMD FP square              FSQRT                           22                   1/19        VMC
  root, Q-form, F64
```

Notes:

1. Floating-point division operations may finish early if the divisor is a power of two.

### 3.18 ASIMD BFloat16 (BF16) instructions

Table 3-17 AArch64 ASIMD BFloat16 (BF16) instructions

```text
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD convert,               BFCVTN,                           4                    2,1       VALU
  F32 to BF16                  BFCVTN2
  ASIMD dot product            BFDOT                           10                     2,1       VMAC,
                                                                                                VALU
  ASIMD matrix                 BFMMLA                      14, 15                  1,1/2        VMAC,
  multiply accumulate                                                                           VALU
  ASIMD multiply               BFMLALB,                          4                    2,1       VMAC
  accumulate long              BFMLALT
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Scalar convert, F32          BFCVT                             4                    2,1       VALU
  to BF16
```

### 3.19 ASIMD miscellaneous instructions

Table 3-18 AArch64 ASIMD miscellaneous instructions

```text
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD bit reverse            RBIT                              3                    2,1       VALU
  ASIMD bitwise                BIF, BIT,                         3                    2,1       VALU
  insert                       BSL
  ASIMD count                  CLS, CLZ,                         3                    2,1       VALU
                               CNT
  ASIMD duplicate,             DUP                               3                      1       VALU
  gen reg
  ASIMD duplicate,             DUP                               3                    2,1       VALU
  element
  ASIMD extract                EXT                               3                    2,1       VALU
  ASIMD extract                XTN                               4                    2,1       VALU
  narrow
  ASIMD extract                SQXTN(2),                         4                    2,1       VALU
  narrow, saturating           SQXTUN(2),
                               UQXTN(2)
  ASIMD insert,                INS                               4                    2,1       VALU
  element to element
  ASIMD move, FP               FMOV                              3                    2,1       VALU
  immed
  ASIMD move,                  MOVI,                             3                    2,1       VALU
  integer immed                MVNI
  ASIMD reciprocal             FRECPE,                           4                    2,1       VMAC
  estimate, D-form             FRECPX,
  F32 and F64                  FRSQRTE,
                               URECPE,
                               URSQRTE
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD reciprocal             FRECPE,                           4                    2,1       VMAC
  estimate, D-form             FRECPX,
  F16 and Q-form               FRSQRTE,
  F32                          URECPE,
                               URSQRTE
  ASIMD reciprocal             FRECPE,                           4                    2,1       VMAC
  estimate, Q-form             FRECPX,
  F16                          FRSQRTE,
                               URECPE,
                               URSQRTE
  ASIMD reciprocal             FRECPS,                           4                    2,1       VMAC
  step                         FRSQRTS
  ASIMD reverse                REV16,                            3                    2,1       VALU
                               REV32,
                               REV64
  ASIMD table                  TBL                               4                    2,1       VALU
  lookup, 1 table regs
  ASIMD table                  TBL                               8                   2/5        VALU
  lookup, 2 table regs
  ASIMD table                  TBL                             12                    1/5        VALU
  lookup, 3 table regs
  ASIMD table                  TBL                             16                    1/9        VALU
  lookup, 4 table regs
  ASIMD table lookup           TBX                               8                   2/5        VALU
  extension, 1 table
  reg
  ASIMD table lookup           TBX                             12                    1/5        VALU
  extension, 2 table
  reg
  ASIMD table lookup           TBX                             16                    1/9        VALU
  extension, 3 table
  reg
  ASIMD table lookup           TBX                             20                   1/13        VALU
  extension, 4 table
  reg
  ASIMD transfer,              UMOV,                             3                      1       VALU
  element to gen reg           SMOV
  ASIMD transfer,              INS                               3                      1       VALU
  gen reg to element
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD transpose              TRN1,                             3                    2,1       VALU
                               TRN2
  ASIMD unzip/zip              UZP1,                             3                    2,1       VALU
                               UZP2, ZIP1,
                               ZIP2
```

### 3.20 ASIMD load instructions

The latencies shown assume the memory access hits in the Level 1 Data Cache.

Base register updates are done in parallel to the operation.

Table 3-19 AArch64 load instructions

```text
  Instruction Group            AArch64                Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD load, 1                LD1                               3                      2       Load/Store,
  element, multiple, 1                                                                          Load
  reg, D-form
  ASIMD load, 1                LD1                               3                      2       Load/Store,
  element, multiple, 1                                                                          Load
  reg, Q-form
  ASIMD load, 1                LD1                               3                      1       Load/Store
  element, multiple, 2
  reg, D-form
  ASIMD load, 1                LD1                               3                      1       Load/Store
  element, multiple, 2
  reg, Q-form
  ASIMD load, 1                LD1                               4                   1/2        Load/Store
  element, multiple, 3
  reg, D-form
  ASIMD load, 1                LD1                               4                   1/2        Load/Store
  element, multiple, 3
  reg, Q-form
  ASIMD load, 1                LD1                               4                   1/2        Load/Store
  element, multiple, 4
  reg, D-form
  ASIMD load, 1                LD1                               4                   1/2        Load/Store
  element, multiple, 4
  reg, Q-form
  Instruction Group            AArch64                Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD load, 1                LD1                               3                      2       Load/Store
  element, one lane,
  B/H/S
  ASIMD load, 1                LD1                               3                      2       Load/Store,
  element, one lane, D                                                                          Load
  ASIMD load, 1                LD1R                              3                      2       Load/Store,
  element, all lanes,                                                                           Load
  D-form, B/H/S
  ASIMD load, 1                LD1R                              3                      2       Load/Store,
  element, all lanes,                                                                           Load
  D-form, D
  ASIMD load, 1                LD1R                              3                      2       Load/Store,
  element, all lanes,                                                                           Load
  Q-form
  ASIMD load, 2                LD2                               4                     1        Load/Store
  element, multiple,
  D-form, B/H/S
  ASIMD load, 2                LD2                               4                   1/2        Load/Store
  element, multiple,
  Q-form, B/H/S
  ASIMD load, 2                LD2                               4                      1       Load/Store
  element, multiple,
  Q-form, D
  ASIMD load, 2                LD2                               4                   1/2        Load/Store
  element, one lane,
  B/H
  ASIMD load, 2                LD2                               4                   1/2        Load/Store
  element, one lane, S
  ASIMD load, 2                LD2                               4                   1/2        Load/Store
  element, one lane, D
  ASIMD load, 2                LD2R                              3                      1       Load/Store
  element, all lanes,
  D-form, B/H/S
  ASIMD load, 2                LD2R                              3                      1       Load/Store
  element, all lanes,
  D-form, D
  ASIMD load, 2                LD2R                              3                      1       Load/Store
  element, all lanes,
  Q-form
  Instruction Group            AArch64                Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD load, 3                LD3                               5                   1/3        Load/Store
  element, multiple,
  D-form, B/H/S
  ASIMD load, 3                LD3                               5                   1/3        Load/Store
  element, multiple,
  Q-form, B/H/S
  ASIMD load, 3                LD3                               5                   1/3        Load/Store
  element, multiple,
  Q-form, D
  ASIMD load, 3                LD3                               5                   1/3        Load/Store
  element, one lane,
  B/H
  ASIMD load, 3                LD3                               5                   1/3        Load/Store
  element, one lane, S
  ASIMD load, 3                LD3                               5                   1/3        Load/Store
  element, one lane, D
  ASIMD load, 3                LD3R                              4                   1/2        Load/Store
  element, all lanes,
  D-form, B/H/S
  ASIMD load, 3                LD3R                              4                   1/2        Load/Store
  element, all lanes,
  D-form, D
  ASIMD load, 3                LD3R                              4                   1/2        Load/Store
  element, all lanes,
  Q-form, B/H/S
  ASIMD load, 3                LD3R                              4                   1/2        Load/Store
  element, all lanes,
  Q-form, D
  ASIMD load, 4                LD4                               5                   1/3        Load/Store
  element, multiple,
  D-form, B/H/S
  ASIMD load, 4                LD4                               5                   1/3        Load/Store
  element, multiple,
  Q-form, B/H/S
  ASIMD load, 4                LD4                               5                   1/4        Load/Store
  element, multiple,
  Q-form, D
  ASIMD load, 4                LD4                               6                   1/4        Load/Store
  element, one lane,
  B/H
  Instruction Group            AArch64                Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD load, 4                LD4                               6                   1/4        Load/Store
  element, one lane, S
  ASIMD load, 4                LD4                               6                   1/4
  element, one lane, D
  ASIMD load, 4                LD4R                              4                   1/2        Load/Store
  element, all lanes,
  D-form, B/H/S
  ASIMD load, 4                LD4R                              4                   1/2        Load/Store
  element, all lanes,
  D-form, D
  ASIMD load, 4                LD4R                              4                   1/2        Load/Store
  element, all lanes,
  Q-form, B/H/S
  ASIMD load, 4                LD4R                              4                   1/2        Load/Store
  element, all lanes,
  Q-form, D
```

### 3.21 ASIMD store instructions

Base register updates are done in parallel to the operation.

Table 3-20 AArch64 ASIMD store instructions

```text
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD store, 1               ST1                                -                     1       Load/Store
  element, multiple, 1
  reg, D-form
  ASIMD store, 1               ST1                                -                     1       Load/Store
  element, multiple, 1
  reg, Q-form
  ASIMD store, 1               ST1                                -                     1       Load/Store
  element, multiple, 2
  reg, D-form
  ASIMD store, 1               ST1                                -                  1/2        Load/Store
  element, multiple, 2
  reg, Q-form
  ASIMD store, 1               ST1                                -                  1/3        Load/Store
  element, multiple, 3
  reg, D-form
  Instruction Group            AArch64                Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  ASIMD store, 1               ST1                                -                  1/3        Load/Store
  element, multiple, 3
  reg, Q-form
  ASIMD store, 1               ST1                                -                  1/2        Load/Store
  element, multiple, 4
  reg, D-form
  ASIMD store, 1               ST1                                -                  1/4        Load/Store
  element, multiple, 4
  reg, Q-form
  ASIMD store, 1               ST1                                -                     1       Load/Store
  element, one lane,
  B/H/S
  ASIMD store, 1               ST1                                -                     1       Load/Store
  element, one lane, D
  ASIMD store, 2               ST2                                -                     1       Load/Store
  element, multiple,
  D-form, B/H/S
  ASIMD store, 2               ST2                                -                  1/2        Load/Store
  element, multiple,
  Q-form, B/H/S
  ASIMD store, 2               ST2                                -                  1/2        Load/Store
  element, multiple,
  Q-form, D
  ASIMD store, 2               ST2                                -                     1       Load/Store
  element, one lane,
  B/H/S
  ASIMD store, 2               ST2                                -                     1       Load/Store
  element, one lane, D
  ASIMD store, 3               ST3                                -                  1/4        Load/Store
  element, multiple,
  D-form, B/H/S
  ASIMD store, 3               ST3                                -                  1/6        Load/Store
  element, multiple,
  Q-form, B/H/S
  ASIMD store, 3               ST3                                -                  1/3        Load/Store
  element, multiple,
  Q-form, D
  ASIMD store, 3               ST3                                -                  1/2        Load/Store
  element, one lane,
  B/H/S
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  ASIMD store, 3                ST3                                -                  1/2        Load/Store
  element, one lane, D
  ASIMD store, 4                ST4                                -                  1/4        Load/Store
  element, multiple,
  D-form, B/H/S
  ASIMD store, 4                ST4                                -                  1/8        Load/Store
  element, multiple,
  Q-form, B/H/S
  ASIMD store, 4                ST4                                -                  1/4        Load/Store
  element, multiple,
  Q-form, D
  ASIMD store, 4                ST4                                -                  1/2        Load/Store
  element, one lane,
  B/H/S
  ASIMD store, 4                ST4                                -                 1/2         Load/Store
  element, one lane, D
```

### 3.22 Cryptography extensions

Table 3-21 AArch64 Cryptography instructions

```text
  Instruction Group             AArch64                    Exec            Execution             Utilized
                                Instruction                Laten           Throughput            Pipeline
                                                           cy
  Crypto AES ops                AESD, AESE,                       3                    2,1       Crypto
                                AESIMC,
                                AESMC
  Crypto polynomial             PMULL (2)                         4                      2       VMC
  (64x64) multiply
  long
  Crypto SHA1 hash              SHA1H                             3                 1,1/2        VALU
  acceleration op
  Crypto SHA1 hash              SHA1C,                            4                      2       VMC
  acceleration ops              SHA1M,
                                SHA1P
  Crypto SHA1                   SHA1SU0,                          3                      2       VMC
  schedule                      SHA1SU1
  acceleration ops
  Instruction Group             AArch64                    Exec            Execution             Utilized
                                Instruction                Laten           Throughput            Pipeline
                                                           cy
  Crypto SHA256                 SHA256H,                          4                      2       VMC
  hash acceleration             SHA256H2
  ops
  Crypto SHA256                 SHA256SU0,                        4                      2       VMC
  schedule                      SHA256SU1
  acceleration ops
  Crypto SHA512                 SHA512H,                          9                   1/9        VMC
  hash acceleration             SHA512H2,
  ops                           SHA512SU0,
                                SHA512SU1
  Crypto SHA3 ops               BCAX, EOR3,                       3                    2,1     VALU
                                XAR                               4
  Crypto SHA3 ops               RAX1                              9                   1/9      VMC
  RAX1
  Crypto SM3 ops                SM3PARTW1,                        9                   1/9        VMC
                                SM3PARTW2,
                                SM3SS1,
                                SM3TT1A,
                                SM3TT1B,
                                SM3TT2A,
                                SM3TT2B
  Crypto SM4 ops                SM4E,                             9                   1/9        VMC
                                SM4EKEY
```

### 3.23 CRC

Table 3-22 AArch64 CRC instructions

```text
  Instruction Group             AArch64                Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  CRC checksum ops              CRC32,                            2                      1       MAC
                                CRC32C
```

### 3.24 SVE Predicate instructions

Table 3-23 SVE Predicate instructions

```text
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Loop control, based          BRKA,                             2                      1       ALU
  on predicate                 BRKB
  Loop control, based          BRKAS,                            2                      1       ALU
  on predicate and             BRKBS
  flag setting
  Loop control,                BRKN,                             2                      1       ALU
  propagating                  BRKPA,
                               BRKPB
  Loop control,                BRKNS,                            2                      1       ALU
  propagating and flag         BRKPAS,
  setting                      BRKPBS
  Loop control, based          WHILEGE,                          2                      1       ALU
  on GPR                       WHILEGT,
                               WHILEHI,
                               WHILEHS,
                               WHILELE,
                               WHILELO,
                               WHILELS,
                               WHILELT,
                               WHILERW,
                               WHILEWR
  Loop terminate [1]           CTERMEQ,                          1                      1       ALU
                               CTERMNE
  Predicate counting           ADDPL,                            1                      2       ALU
  scalar                       ADDVL,
                               RDVL,
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
                               CNTB,                             1                      1
                               CNTH,
                               CNTW,
                               CNTD,
                               DECB,
                               DECH,
                               DECW,
                               DECD,
                               INCB,
                               INCH,
                               INCW,
                               INCD,

                               SQDECB,                           5                      1       ALU
                               SQDECH,
                               SQDECW,
                               SQDECD,
                               SQINCB,
                               SQINCH,
                               SQINCW,
                               SQINCD,
                               UQDECB,
                               UQDECH,
                               UQDECW,
                               UQDECD,
                               UQINCB,
                               UQINCH,
                               UQINCW,
                               UQINCD
  Predicate counting           CNTP,                             6                      1       ALU
  scalar, active               DECP, INCP
  predicate
  Predicate counting           SQDECP,                           9              1/3,1/6         ALU
  scalar, active               SQINCP,
  predicate,                   UQDECP,
  saturating, 64-bit           UQINCP
  Predicate counting           SQDECP,                           3              1/3,1/6         ALU
  scalar, active               SQINCP,
  predicate,
                               UQDECP,                           9
  saturating, 32-bit
                               UQINCP
  Instruction Group                SVE                    Exec                Execution             Utilized
                                   Instruction            Latency             Throughput            Pipeline
  Predicate counting               SQDECP,                           4                    2,1       ALU
  vector, active                   SQINCP,
  predicate,                       UQDECP,
  saturating                       UQINCP
  Predicate logical                AND, BIC,                         2                      1       ALU
                                   EOR, MOV,
                                   NAND,
                                   NOR, NOT,
                                   ORN, ORR
  Predicate logical,               ANDS, BICS,                       2                      1       ALU
  flag setting                     EORS,
                                   MOV,
                                   NANDS,
                                   NORS,
                                   NOTS,
                                   ORNS,
                                   ORRS
  Predicate reverse                REV                               2                      1       ALU
  Predicate select                 SEL                               2                      1       ALU
  Predicate set                    PFALSE,                           2                      1       ALU
                                   PTRUE
  Predicate                        PTRUES                            2                     1        ALU
  set/initialize, set
  flags
  Predicate find                   PFIRST,                           2                      1       ALU
  first/next                       PNEXT
  Predicate test                   PTEST                             2                      1       ALU

  Predicate transpose              TRN1,                             2                      1       ALU
                                   TRN2
  Predicate unpack                 PUNPKHI,                          2                      1       ALU
  and widen                        PUNPKLO
  Predicate zip/unzip              ZIP1, ZIP2,                       2                      1       ALU
                                   UZP1,
                                   UZP2
```

Notes:

1. Instructions with dependencies may be co-issue.

### 3.25 SVE Integer instructions

Table 3-24 SVE integer instructions

```text
  Instruction Group            SVE Instruction              Exec           Execution            Utilized
                                                            Laten          Throughp             Pipeline
                                                            cy             ut
  Arithmetic, absolute         SABD, UABD                          3                  2,1       VALU
  diff
  Arithmetic, absolute         SABA, UABA                          6            1/2,1/4         VALU
  diff accum
  Arithmetic, absolute         SABALB,                             6            1/2,1/4         VALU
  diff accum long              SABALT,
                               UABALB,
                               UABALT
  Arithmetic, absolute         SABDLB,                             3                  2,1       VALU
  diff long                    SABDLT,
                               UABDLB,
                               UABDLT
  Arithmetic, basic            ABS,                                3                  2,1       VALU
                               ADD, ADR,
                               CNOT, NEG,
                               SHADD, SHSUB,
                               SHSUBR,
                               SRHADD, SUB,
                               UADDWB,
                               UADDWT,
                               UHADD,
                               UHSUB,
                               UHSUBR,
                               URHADD,
                               SUBHNB,                             4
                               SUBHNT,
                               SUBR,
                               USUBWB,
                               USUBWT
  Instruction Group              SVE Instruction              Exec           Execution            Utilized
                                                              Laten          Throughp             Pipeline
                                                              cy             ut
  Arithmetic, basic              SADDLB,                             4                  2,1       VALU
                                 SADDLBT,
                                 SADDLT,
                                 SADDWB,
                                 SADDWT,
                                 SSUBLB,
                                 SSUBLBT,
                                 SSUBLT,
                                 SSUBLTB,
                                 SSUBWB,
                                 SSUBWT,
                                 UADDLB,
                                 UADDLT,
                                 USUBLB,
                                 USUBLT,
  Arithmetic, complex            ADDHNB,                             4                  2,1       VALU
                                 ADDHNT,
                                 SQABS, SQADD,
                                 SQNEG, SQSUB,
                                 SQSUBR,
                                 SUQADD,
                                 UQADD,
                                 UQSUB,
                                 UQSUBR,
                                 USQADD,
                                 RADDHNB,                            8            2/5,1/5
                                 RADDHNT,
                                 RSUBHNB,
                                 RSUBHNT

  Arithmetic, large              ADCLB, ADCLT,                       4                  2,1       VALU
  integer                        SBCLB, SBCLT
  Arithmetic, pairwise           ADDP                                3                  2,1       VALU
  add
  Arithmetic, pairwise           SADALP,                             7            2/5,1/5         VALU
  add and accum long             UADALP
  Arithmetic, shift              ASR, ASRR, LSL,                     3                  2,1       VALU
                                 LSLR, LSR, LSRR
  Arithmetic, shift and          USRA                                4                  2,1       VALU
  accumulate
  Instruction Group              SVE Instruction              Exec           Execution            Utilized
                                                              Laten          Throughp             Pipeline
                                                              cy             ut
  Arithmetic, shift and          SRSRA,                              7            2/5,1/5         VALU
  accumulate complex             URSRA,
                                 SSRA                                4                  2,1
  Arithmetic, shift by           SHRNB, SHRNT,                       3                  2,1       VALU
  immediate                      SSHLLB,
                                 SSHLLT,
                                 USHLLB,
                                 USHLLT
  Arithmetic, shift by           SLI, SRI                            3                  2,1       VALU
  immediate and
  insert
  Arithmetic, shift              RSHRNB,                             4                  2,1       VALU
  complex                        RSHRNT,
                                 SQRSHL,
                                 SQRSHLR,
                                 SQRSHRNB,
                                 SQRSHRNT,
                                 SQRSHRUNB,
                                 SQRSHRUNT,
                                 SQSHL,
                                 SQSHLR,
                                 SQSHLU,
                                 SQSHRNB,
                                 SQSHRNT,
                                 SQSHRUNB,
                                 SQSHRUNT,
                                 UQRSHL,
                                 UQRSHLR,
                                 UQRSHRNB,
                                 UQRSHRNT,
                                 UQSHL,
                                 UQSHLR,
                                 UQSHRNB,
                                 UQSHRNT
  Arithmetic, shift              ASRD                                4                  2,1       VALU
  right for divide
  Arithmetic, shift              SRSHL, SRSHLR,                      4                  2,1       VALU
  rounding                       SRSHR, URSHL,
                                 URSHLR,
                                 URSHR
  Bit manipulation (B)           BDEP, BEXT,                       14                 1/14        VMC
                                 BGRP
  Instruction Group            SVE Instruction              Exec           Execution            Utilized
                                                            Laten          Throughp             Pipeline
                                                            cy             ut
  Bit manipulation (H)         BDEP, BEXT,                       22                 1/22        VMC
                               BGRP
  Bit manipulation (S)         BDEP, BEXT,                       38                 1/38        VMC
                               BGRP
  Bit manipulation (D)         BDEP, BEXT,                       70                 1/70        VMC
                               BGRP
  Bitwise select               BSL, BSL1N,                         3                  2,1       VALU
                               BSL2N, NBSL
  Count/reverse bits           CLS, CLZ, RBIT                      3                  2,1       VALU
  Count (B,H)                  CNT                                 3                  2,1       VALU
  Count (S)                    CNT                                 8            2/5,1/5         VALU
  Count (D)                    CNT                               12           1/5,1/10          VALU
  Broadcast logical            DUPM, MOV                           4                  2,1       VALU
  bitmask immediate
  to vector
  Compare and set              CMPEQ,                              5                  2,1       VALU
  flags                        CMPGE,
                               CMPGT, CMPHI,
                               CMPHS, CMPLE,
                               CMPLO, CMPLS,
                               CMPLT, CMPNE
  Complex add                  CADD                                3                  2,1       VALU
  Complex add                  SQCADD                              4                  2,1       VALU
  saturating
  Complex dot                  CDOT                                4                  2,1       VMAC
  product 8-bit
  element
  Complex dot                  CDOT                                4                  2,1       VMAC
  product 16-bit
  element
  Complex multiply-            CMLA                                4                  2,1       VMAC
  add B, H, S element
  size
  Complex multiply-            CMLA                                4                  2,1       VMAC
  add D element size
  Instruction Group              SVE Instruction              Exec           Execution            Utilized
                                                              Laten          Throughp             Pipeline
                                                              cy             ut
  Conditional extract            CLASTA,                             8                  1,1       VALU
  operations, general            CLASTB
  purpose register
  Conditional extract            CLASTA,                             4                  2,1       VALU
  operations,                    CLASTB,
  SIMD&FP scalar                 COMPACT,
  and vector forms               SPLICE
  Convert to floating            SCVTF, UCVTF                        4                  2,1       VALU
  point, 64b to float
  or convert to double
  Convert to floating            SCVTF, UCVTF                        4                  2,1       VALU
  point, 32b to single
  or half
  Convert to floating            SCVTF, UCVTF                        4                  2,1       VALU
  point, 16b to half
  Copy, scalar                   CPY                                 3                  2,1       VALU
  Copy, scalar                   CPY                                 3                  2,1       VALU
  SIMD&FP or imm
  Divides, 32 bit                SDIV, SDIVR,                      15                 1/12        VMC
                                 UDIV, UDIVR
  Divides, 64 bit                SDIV, SDIVR,                      26                 1/23        VMC
                                 UDIV, UDIVR
  Dot product, 8 bit             SDOT, UDOT                          4                  2,1       VMAC
  Dot product, 8 bit,            SUDOT, USDOT                        4                  2,1       VMAC
  using signed and
  unsigned integers
  Dot product, 16 bit            SDOT, UDOT                          4                  2,1       VMAC
  Duplicate,                     DUP, MOV                            3                  2,1       VALU
  immediate and
  indexed form
  Duplicate, indexed >           DUP                                 3                  2,1       VALU
  elem
  Duplicate, scalar              DUP, MOV                            3                  2,1       VALU
  form
  Extend, sign or zero           SXTB, SXTH,                         3                  2,1       VALU
                                 SXTW, UXTB,
                                 UXTH, UXTW
  Instruction Group              SVE Instruction              Exec           Execution            Utilized
                                                              Laten          Throughp             Pipeline
                                                              cy             ut
  Extract                        EXT                                 3                  2,1       VALU
  Extract narrow                 SQXTNB,                             4                  2,1       VALU
  saturating                     SQXTNT,
                                 SQXTUNB,
                                 SQXTUNT,
                                 UQXTNB,
                                 UQXTNT
  Extract/insert                 LASTA, LASTB,                       4                  2,1       VALU
  operation, SIMD                INSR
  and FP scalar form
  Extract/insert                 LASTA, LASTB,                     8,8            1/3,1/3         VALU0
  operation, scalar
                                 INSR                                4                  2,1
  Histogram                      HISTCNT,                            8                 2/5        VALU0
  operations                     HISTSEG
  Horizontal                     INDEX                               4                  2,1       VMAC
  operations, B, H, S
  form, immediate
  operands only
  Horizontal                     INDEX                               4                  1,1       VMAC
  operations, B, H, S
  form, scalar,
  immediate
  operands)/ scalar
  operands only /
  immediate, scalar
  operands
  Horizontal                     INDEX                               4                  2,1       VMAC
  operations, D form,
  immediate operands
  only
  Horizontal                     INDEX                               4                  1,1       VMAC
  operations, D form,
  scalar, immediate
  operands)/ scalar
  operands only /
  immediate, scalar
  operands
  Logical                        AND, BIC, EON,                      3                  2,1       VALU
                                 EOR, MOV,
                                 NOT, ORN, ORR
  Instruction Group              SVE Instruction              Exec           Execution            Utilized
                                                              Laten          Throughp             Pipeline
                                                              cy             ut
  Logical                        EORBT, EORTB,                       4                  2,1       VALU
  Max/min, basic and             SMAX, SMAXP,                        3                  2,1       VALU
  pairwise                       SMIN, SMINP,
                                 UMAX, UMAXP
                                 UMIN, UMINP
  Matching                       MATCH,                            7,7            1/4,1/4         VALU
  operations                     NMATCH
  Matrix multiply-               SMMLA,                              4                  2,1       VMAC
  accumulate                     UMMLA,
                                 USMMLA
  Move prefix                    MOVPRFX                             3                  2,1       VALU
  Multiply, B, H, S              MUL, SMULH,                         4                  2,1       VMAC
  element size                   UMULH
  Multiply, D element            MUL, SMULH,                         4                  2,1       VMAC
  size                           UMULH
  Multiply long                  SMULLB,                             4                  2,1       VMAC
                                 SMULLT,
                                 UMULLB,
                                 UMULLT
  Multiply                       MLA, MLS                            4                  2,1       VMAC
  accumulate, B, H, S
  element size
  Multiply                       MLA, MLS, MAD,                      4                  2,1       VMAC
  accumulate, D                  MSB,
  element size
  Multiply accumulate            SMLALB,                             4                  2,1       VMAC
  long                           SMLALT,
                                 SMLSLB,
                                 SMLSLT,
                                 UMLALB,
                                 UMLALT,
                                 UMLSLB,
                                 UMLSLT
  Multiply accumulate            SQDMLALB,                           4                  2,1       VMAC
  saturating doubling            SQDMLALT,
  long regular                   SQDMLALBT,
                                 SQDMLSLB,
                                 SQDMLSLT,
                                 SQDMLSLBT
  Instruction Group            SVE Instruction              Exec           Execution            Utilized
                                                            Laten          Throughp             Pipeline
                                                            cy             ut
  Multiply saturating          SQDMULH                             4                  2,1       VMAC
  doubling high, B, H,
  S element size
  Multiply saturating          SQDMULH                             4                  2,1       VMAC
  doubling high, D
  element size
  Multiply saturating          SQDMULLB,                           4                  2,1       VMAC
  doubling long                SQDMULLT
  Multiply saturating          SQRDMLAH,                           4                  1,1       VMAC
  rounding doubling            SQRDMLSH,
  regular/complex              SQRDCMLAH
  accumulate, B, H, S
  element size
  Multiply saturating          SQRDMLAH,                           4                  1,1       VMAC
  rounding doubling            SQRDMLSH,
  regular/complex              SQRDCMLAH
  accumulate, D
  element size
  Multiply saturating          SQRDMULH                            4                  2,1       VMAC
  rounding doubling
  regular/complex, B,
  H, S element size
  Multiply saturating          SQRDMULH                            4                  2,1       VMAC
  rounding doubling
  regular/complex, D
  element size
  Multiply/multiply            PMUL, PMULLB,                       4                  2,1       VALU
  long, (8, 16, 32)            PMULLT
  polynomial
  Multiply/multiply            PMULLB,                             9                 1/9        VMC
  long, (64)                   PMULLT
  polynomial
  Predicate counting           DECH, DECW,                         3                  2,1       VALU
  vector                       DECD, INCH,
                               INCW, INCD
  Instruction Group            SVE Instruction              Exec           Execution            Utilized
                                                            Laten          Throughp             Pipeline
                                                            cy             ut
  Predicate counting           SQDECH,                             4                  2,1       VALU
  vector, saturating           SQDECW,
                               SQDECD,
                               SQINCH,
                               SQINCW,
                               SQINCD,
                               UQDECH,
                               UQDECW,
                               UQDECD,
                               UQINCH,
                               UQINCW,
                               UQINCD
  Reciprocal estimate          URECPE,                             4                  2,1       VMAC
                               URSQRTE
  Reduction,                   SADDV,                              4                    1       VALU0
  arithmetic, B form           UADDV,
                               SMAXV, SMINV,
                               UMAXV, UMINV
  Reduction,                   SADDV,                              4                    1       VALU0
  arithmetic, H form           UADDV,
                               SMAXV, SMINV,
                               UMAXV, UMINV
  Reduction,                   SADDV,                              4                    1       VALU0
  arithmetic, S form           UADDV,
                               SMAXV, SMINV,
                               UMAXV, UMINV
  Reduction, logical           ANDV, EORV,                         4                    1       VALU0
                               ORV

  Reverse, vector              REV, REVB,                        3,3                  2,1       VALU
                               REVH, REVW
  Select, vector form          MOV, SEL                            3                  2,1       VALU
  Table lookup                 TBL                                 4                  2,1       VALU
  Table lookup,                TBL                                 8            2/5,1/5         VALU
  double table
  Table lookup                 TBX                                 4                  2,1       VALU
  extension
  Transpose, vector            TRN1, TRN2                          3                  2,1       VALU
  form
  Instruction Group             SVE Instruction              Exec           Execution            Utilized
                                                             Laten          Throughp             Pipeline
                                                             cy             ut
  Unpack and extend             SUNPKHI,                              4                2,1       VALU
                                SUNPKLO,
                                UUNPKHI,
                                UUNPKLO
  Zip/unzip                     UZP1, UZP2,                           3                2,1       VALU
                                ZIP1, ZIP2
```

### 3.26 SVE FP data processing instructions

Table 3-25 SVE Floating-point instructions

```text
  Instruction Group             SVE                    Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Floating point                FABD, FABS                        4                    2,1       VALU
  absolute
  value/difference
  Floating point                FADD,                             4                    2,1       VALU
  arithmetic                    FADDP,
                                FNEG,
                                FSUB,
                                FSUBR
  Floating point                FADDA                           32                   1/25        VALU
  associative add, F16
  Floating point                FADDA                           16             1/9,1/18          VALU
  associative add, F32
  Floating point                FADDA                             8              2/5,1/5         VALU
  associative add, F64
  Floating point                FACGE,                            4                 1,1/2        VALU
  compare                       FACGT,
                                FACLE,
                                FACLT,
                                FCMEQ,
                                FCMGE,
                                FCMGT,
                                FCMLE,
                                FCMLT,
                                FCMNE,
                                FCMUO
  Floating point                FCADD                             4                    2,1       VALU
  complex add
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Floating point               FCMLA                             4                    2,1       VMAC
  complex multiply
  add
  Floating point               FCVT,                             4                    2,1       VALU
  convert, long or             FCVTLT,
  narrow (F16 to F32           FCVTNT
  or F32 to F16)
  Floating point               FCVT,                             4                    2,1       VALU
  convert, long or             FCVTLT,
  narrow (F16 to F64,          FCVTNT
  F32 to F64, F64 to
  F32 or F64 to F16)
  Floating point               FCVTX,                            4                    2,1       VALU
  convert, round to            FCVTXNT
  odd
  Floating point base2         FLOGB                             4                    2,1       VMAC
  log, F16
  Floating point base2         FLOGB                             4                    2,1       VMAC
  log, F32
  Floating point base2         FLOGB                             4                    2,1       VMAC
  log, F64
  Floating point               FCVTZS,                           4                    2,1       VALU
  convert to integer,          FCVTZU
  F16
  Floating point               FCVTZS,                           4                    2,1       VALU
  convert to integer,          FCVTZU
  F32
  Floating point               FCVTZS,                           4                    2,1       VALU
  convert to integer,          FCVTZU
  F64
  Floating point copy          FCPY,                             3                    2,1       VALU
                               FDUP,
                               FMOV
  Floating point               FDIV,                             8                   1/5        VMC
  divide, F16 1                FDIVR
  Floating point               FDIV,                           13                   1/10        VMC
  divide, F32 1                FDIVR
  Floating point               FDIV,                           22                   1/19        VMC
  divide, F64 1                FDIVR
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Floating point               FMAXP,                            4                    2,1       VALU
  min/max pairwise             FMAXNMP,
                               FMINP,
                               FMINNMP
  Floating point               FMAX,                             4                    2,1       VALU
  min/max                      FMIN,
                               FMAXNM,
                               FMINNM
  Floating point               FSCALE,                           4                    2,1       VMAC
  multiply                     FMUL,
                               FMULX
  Floating point               FMLA,                             4                    2,1       VMAC
  multiply accumulate          FMLS,
                               FMAD,
                               FMSB,
                               FNMAD,
                               FNMLA,
                               FNMLS,
                               FNMSB
  Floating point               FMLALB,                           4                    2,1       VMAC
  multiply add/sub             FMLALT,
  accumulate long              FMLSLB,
                               FMLSLT
  Floating point               FRECPE,                           4                    2,1       VMAC
  reciprocal estimate,         FRECPX,
  F16                          FRSQRTE
  Floating point               FRECPE,                           4                    2,1       VMAC
  reciprocal estimate,         FRECPX,
  F32                          FRSQRTE
  Floating point               FRECPE,                           4                    2,1       VMAC
  reciprocal estimate,         FRECPX,
  F64                          FRSQRTE
  Floating point               FRECPS,                           4                    2,1       VMAC
  reciprocal step              FRSQRTS
  Floating point               FMAXNMV                           4                      1       VALU0
  reduction, F16               FMAXV,
                               FMINNMV,
                               FMINV
  Floating point               FADDV                           12                    1/5        VALU0
  reduction, F16
  Instruction Group             SVE                    Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Floating point                FADDV                             8                   2/5        VALU0
  reduction, F32
  Floating point                FADDV                             4                      2       VALU0
  reduction, F64
  Floating point round          FRINTA,                           4                    2,1       VALU
  to integral, F16              FRINTI,
                                FRINTM,
                                FRINTN,
                                FRINTP,
                                FRINTX,
                                FRINTZ
  Floating point round          FRINTA,                           4                    2,1       VALU
  to integral, F32              FRINTI,
                                FRINTM,
                                FRINTN,
                                FRINTP,
                                FRINTX,
                                FRINTZ
  Floating point round          FRINTA,                           4                    2,1       VALU
  to integral, F64              FRINTI,
                                FRINTM,
                                FRINTN,
                                FRINTP,
                                FRINTX,
                                FRINTZ
  Floating point                FSQRT                             8                   1/5        VMC
  square root, F16
  Floating point                FSQRT                           12                    1/9        VMC
  square root, F32
  Floating point                FSQRT                           22                   1/19        VMC
  square root F64
  Floating point                FEXPA                             4                    2,1       VMAC
  trigonometric
  exponentiation
  Floating point                FTMAD                             4                    2,1       VMAC
  trigonometric
  multiply add
  Floating point                FTSMUL                            4                    2,1       VMAC
  trigonometric
  starting value
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Floating point               FTSSEL                          33                     2,1       VALU
  trigonometric select
  coefficient
```

Notes:

1. Floating-point division operations may finish early if the divisor is a power of two.

### 3.27 SVE BFloat16 (BF16) instructions

Table 3-26 SVE Bfloat16 (BF16) instructions

```text
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Convert, F32 to              BFCVT,                            4                   2, 1       VALU
  BF16                         BFCVTNT
  Dot product                  BFDOT                           10                    2, 1       VMAC,
                                                                                                VALU
  Matrix multiply              BFMMLA                      14, 15                 1, 1/2        VMAC,
  accumulate                                                                                    VALU
  Multiply accumulate          BFMLALB,                          4                   2, 1       VMAC
  long                         BFMLALT
```

### 3.28 SVE Load instructions

The latencies shown in Table 3-27 assume the memory access hits in the Level 1 Data Cache.

Base register updates are done in parallel to the operation.

Table 3-27 SVE Load instructions

```text
  Instruction Group            SVE                    Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Load vector                  LDR                               3                      2       Load/Store,
                                                                                                Load
  Load predicate               LDR                               3                      1       Load/Store
  Instruction Group            SVE                    Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Contiguous load,             LD1B,                             3                      2       Load/Store,
  scalar + imm                 LD1D,                                                            Load
                               LD1H,
                               LD1W,
                               LD1SB,
                               LD1SH,
                               LD1SW,
  Contiguous load,             LD1B,                             3                      2       Load/Store,
  scalar + scalar              LD1D,                                                            Load
                               LD1H,
                               LD1W,
                               LD1SB,
                               LD1SH
                               LD1SW
  Contiguous load              LD1RB,                            3                      2       Load/Store,
  broadcast, scalar +          LD1RH,                                                           Load
  imm                          LD1RD,
                               LD1RW,
                               LD1RSB,
                               LD1RSH,
                               LD1RSW,
                               LD1RQB,
                               LD1RQD,
                               LD1RQH,
  Contiguous load              LD1RQB,                           3                      2       Load/Store,
  broadcast, scalar +          LD1RQD,                                                          Load
  scalar                       LD1RQH,
                               LD1RQW
  Non temporal load,           LDNT1B,                           3                      2       Load/Store,
  scalar + imm                 LDNT1D,                                                          Load
                               LDNT1H,
                               LDNT1W
  Non temporal load,           LDNT1B,                           3                      2       Load/Store,
  scalar + scalar              LDNT1D,                                                          Load
                               LDNT1H
                               LDNT1W
  Non temporal                 LDNT1B,                           9                   1/9        Load/Store
  gather load, vector          LDNT1H,
  + scalar 32-bit              LDNT1W,
  element size                 LDNT1SB,
                               LDNT1SH
  Instruction Group            SVE                    Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Non temporal                 LDNT1B,                           7                   1/7        Load/Store
  gather load, vector          LDNT1D,
  + scalar 64-bit              LDNT1H,
  element size                 LDNT1W,
                               LDNT1SB,
                               LDNT1SH,
                               LDNT1SW
  Contiguous first             LDFF1B,                           3                      2       Load/Store,
  faulting load, scalar        LDFF1D,                                                          Load
  + scalar                     LDFF1H,
                               LDFF1W,
                               LDFF1SB,
                               LDFF1SD,
                               LDFF1SH
                               LDFF1SW
  Contiguous non               LDNF1B,                           3                      2       Load/Store,
  faulting load, scalar        LDNF1D,                                                          Load
  + imm                        LDNF1H,
                               LDNF1W,
                               LDNF1SB,
                               LDNF1SH,
                               LDNF1SW
  Contiguous Load              LD2B,                             3                      1       Load/Store
  two structures to            LD2D,
  two vectors, scalar          LD2H,
  + imm                        LD2W
  Contiguous Load              LD2B,                             3                   1/2        Load/Store
  two structures to            LD2D,
  two vectors, scalar          LD2H,
  + scalar                     LD2W
  Contiguous Load              LD3B,                             5                   1/3        Load/Store
  three structures to          LD3D,
  three vectors, scalar        LD3H,
  + imm                        LD3W
  Contiguous Load              LD3B,                             5                   1/4        Load/Store
  three structures to          LD3D,
  three vectors, scalar        LD3H,
  + scalar                     LD3W
  Contiguous Load              LD4B,                             5                   1/3        Load/Store
  four structures to           LD4D,
  four vectors, scalar         LD4H
  + imm                        LD4W
  Instruction Group            SVE                    Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Contiguous Load              LD4B,                             5                   1/4        Load/Store
  four structures to           LD4D,
  four vectors, scalar         LD4H,
  + scalar                     LD4W
  Gather load, vector          LD1B,                             9                   1/9        Load/Store
  + imm, 32-bit                LD1H,
  element size                 LD1W,
                               LD1SB,
                               LD1SH,
                               LD1SW,
                               LDFF1B,
                               LDFF1H,
                               LDFF1W,
                               LDFF1SB,
                               LDFF1SH,
                               LDFF1SW
  Gather load, vector          LD1B,                             7                   1/7        Load/Store
  + imm, 64-bit                LD1D,
  element size                 LD1H,
                               LD1W,
                               LD1SB,
                               LD1SH,
                               LD1SW,
                               LDFF1B,
                               LDFF1D
                               LDFF1H,
                               LDFF1W,
                               LDFF1SB,
                               LDFF1SH,
                               LDFF1SW
  Gather load, 32-bit          LD1H,                             7                   1/7        Load/Store
  scaled offset                LD1W,
                               LDFF1H,
                               LDFF1SH,
                               LDFF1W
  Instruction Group            SVE                    Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Gather load, 32-bit          LD1B,                             7                   1/7        Load/Store
  unpacked unscaled            LD1D,
  offset
                               LD1H,
                               LD1W,
                               LDFF1B,
                               LDFF1D,
                               LDFF1H,
                               LDFF1SB,
                               LDFF1SH,
                               LDFF1SW,
                               LDFF1W
  Gather load, 32-bit          LD1B,                             7                   1/7        Load/Store
  unscaled offset              LD1H,
                               LD1W,
                               LDFF1B,
                               LDFF1H,
                               LDFF1SB,
                               LDFF1SH,
                               LDFF1W
  Gather load, 32-bit          LD1D,                             7                   1/7        Load/Store
  unpacked scaled              LD1H,
  offset
                               LD1W,
                               LDFF1D,
                               LDFF1H,
                               LDFF1SH,
                               LDFF1SW,
                               LDFF1W
  Instruction Group            SVE                    Load                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Gather load, 64-bit          LD1B,                             7                   1/7        Load/Store
  unscaled offset              LD1D,
                               LD1H,
                               LD1W,
                               LDFF1B,
                               LDFF1D,
                               LDFF1H,
                               LDFF1SB,
                               LDFF1SH,
                               LDFF1SW,
                               LDFF1W
  Gather load, 64-bit          LD1D,                             7                   1/7        Load/Store
  scaled offset                LD1H,
                               LD1W,
                               LDFF1D,
                               LDFF1H,
                               LDFF1SH,
                               LDFF1SW,
                               LDFF1W
```

### 3.29 SVE Store instructions

Base register updates are done in parallel to the operation.

Table 3-28 SVE Store instructions

```text
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instructions           Latency             Throughput            Pipeline
  Store from                   STR                                -                     1       Load/Store
  predicate reg
  Store from vector            STR                                -                     1       Load/Store
  reg
  Contiguous store,            ST1B, ST1H,                        -                     1       Load/Store
  scalar + imm                 ST1D,
                               ST1W
  Contiguous store,            ST1H,ST1B,                         -                     1       Load/Store
  scalar + scalar              ST1D,
                               ST1W
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instructions           Latency             Throughput            Pipeline
  Contiguous store             ST2B, ST2H,                        -                  1/2        Load/Store
  two structures from          ST2D,
  two vectors, scalar          ST2W
  + imm
  Contiguous store             ST2H, ST2B,                        -                  1/2        Load/Store
  two structures from          ST2D,
  two vectors, scalar          ST2W
  + scalar
  Contiguous store             ST3B, ST3H,                        -                  1/6        Load/Store
  three structures             ST3W
  from three vectors,
                               ST3D                                                  1/3        Load/Store
  scalar + imm
  Contiguous store             ST3B, ST3H,                        -                  1/6        Load/Store
  three structures             ST3W
  from three vectors,
                               ST3D                               -                  1/3        Load/Store
  scalar + scalar
  Contiguous store             ST4B, ST4H,                        -                  1/8        Load/Store
  four structures from         ST4W
  four vectors, scalar
                               ST4D                                                  1/4        Load/Store
  + imm
  Contiguous store             ST4B, ST4H,                        -                  1/8        Load/Store
  four structures from         ST4W
  four vectors, scalar
  + scalar                     ST4D                               -                  1/4        Load/Store

  Non temporal store,          STNT1B,                            -                     1       Load/Store
  scalar + imm                 STNT1D,
                               STNT1H,
                               STNT1W
  Non temporal store,          STNT1H,                            -                     1       Load/Store
  scalar + scalar              STNT1B,
                               STNT1D,
                               STNT1W
  Scatter non                  STNT1B,                            -                  1/9        Load/Store
  temporal store,              STNT1H,
  vector + scalar 32-          STNT1W
  bit element size
  Scatter non                  STNT1B,                            -                  1/7        Load/Store
  temporal store,              STNT1D,
  vector + scalar 64-          STNT1H,
  bit element size             STNT1W
  Instruction Group             SVE                    Exec                Execution             Utilized
                                Instructions           Latency             Throughput            Pipeline
  Scatter store vector          ST1B, ST1H,                        -                  1/9        Load/Store
  + imm 32-bit                  ST1W
  element size
  Scatter store vector          ST1B, ST1D,                        -                  1/7        Load/Store
  + imm 64-bit                  ST1H,
  element size                  ST1W
  Scatter store, 32-bit         ST1H,                              -                  1/8        Load/Store
  scaled offset                 ST1W
  Scatter store, 32-bit         ST1B, ST1D,                        -                  1/8        Load/Store
  unpacked unscaled             ST1H,
  offset                        ST1W
  Scatter store, 32-bit         ST1D,                              -                  1/8        Load/Store
  unpacked scaled               ST1H,
  offset                        ST1W
  Scatter store, 32-bit         ST1B, ST1H,                        -                  1/8        Load/Store
  unscaled offset               ST1W
  Scatter store, 64-bit         ST1D,                              -                  1/8        Load/Store
  scaled offset                 ST1H,
                                ST1W
  Scatter store, 64-bit         ST1B, ST1D,                        -                  1/8        Load/Store
  unscaled offset               ST1H,
                                ST1W
```

### 3.30 SVE Miscellaneous instructions

Table 3-29 SVE Miscellaneous instructions

```text
  Instruction Group             SVE                    Exec                Execution             Utilized
                                Instruction            Latency             Throughput            Pipeline
  Read first fault              RDFFR                             1                      1       Load/Store
  register,
  unpredicated
  Read first fault              RDFFR                             3                      1       Load/Store
  register, predicated
  Read first fault              RDFFRS                            3                      1       Load/Store
  register and set
  flags
  Set first fault               SETFFR                            1                      1       Load/Store
  register
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instruction            Latency             Throughput            Pipeline
  Write to first fault         WRFFR                             1                      1       Load/Store
  register
```

### 3.31 SVE Cryptography instructions

Table 3-30 SVE cryptography instructions

```text
  Instruction Group            SVE                    Exec                Execution             Utilized
                               Instructions           Latency             Throughput            Pipeline
  Crypto AES ops               AESD, AESE,                       3                    2,1       Crypto
                               AESIMC,
                               AESMC
  Crypto SHA3 ops              BCAX,                             4                    2,1       VALU
                               EOR3, XAR
  Crypto SHA3 ops              RAX1                              9                   1/9        VMC
  RAX1
  Crypto SM4 ops               SM4E,                             9                   1/9        VMC
                               SM4EKEY
```

## 4 Special considerations

### 4.1 Issue constraints

The issue queue has space for three instructions that support a maximum of (excluding
Floating-Point. Predicate, SIMD, SVE register accesses):

- Four general purpose destination registers

- Six general purpose source registers

An instruction will occupy two entries when it has either:

- Three or more general purpose destination registers

- Three or more general purpose source registers

An instruction will stall if insufficient space is available in the issue queue.

AES instructions will stall until there is at least one other instruction available to be issued (see
4.2 Instruction fusion).

A maximum of three issue queue entries can be co-issued per cycle (ignoring hazards)
consisting of at most:

- Two ALU instructions

- Two load instructions

- One store instruction

- Two VPU data processing instructions

Multicycle entries disable co-issuing for all cycles of the operation but the last.

The following are multicycle:

- Atomic instructions with Acquire or Release semantics

- Loads that load more than 256-bit of data

- Stores that store more than 128-bits of data

- Stores with Release semantics

- RDFFRS instructions

### 4.2 Instruction fusion

Cortex-A520 Core can accelerate key instruction pairs in an operation called fusion.

The following instruction pairs can be fused for increased execution efficiency:

- 'AESE + AESMC' and 'AESD + AESIMC' (see 4.13)

- MOVPRFX fusion: Cortex-A520 Core implements instruction fusion for MOVPRFX
instructions followed by SVE data processing instructions in all cases where the
instruction pair is defined as architecturally predictable other than those listed below,
and the fused pair will execute with the latency of the SVE data processing instruction.
Due to microarchitectural limitations, the following instructions will not fuse with an
unpredicated MOVPRFX: FCMLA, FMAD, FMLA, FMLS, FNMAD, FNMLA, FNMLS,
FNMSB, MAD, MLA, MLS, MSB, UDOT, BFMLALB, BFMLALT, SMMLA, UMMLA,
USMMLA, USDOT, SUDOT.
The following instructions will not fuse with a predicated or unpredicated MOVPRFX:
CNT, SABA, SABALB, SABALT, UABA, UABALB, UABALT, URSRA.

### 4.3 Branch instruction alignment

Branch instruction and branch target instruction alignment and density can affect
performance.

For best case performance, avoid placing more than one conditional branch instructions within
an aligned 16-byte instruction memory region.

### 4.4 Load / Store Alignment

The Armv8-A architecture allows many types of load and store accesses to be arbitrarily
aligned. Cortex-A520 Core handles most unaligned accesses without performance penalties.
However, there are cases which could reduce bandwidth or incur additional latency, as
described below.

- Quad-word load operations that are not 4-byte aligned

- Load operations that cross a 32-byte boundary

- Store operations that cross a 16-byte boundary

### 4.5 A64 low latency pointer forwarding

In the A64 instruction set the following pointer sequence is expected to be common to
generate load-store addresses:

```asm
adrp x0, <const>
ldrp x0, [x0, #lo12 <const>]
```

In Cortex-A520 Core, there are dedicated forwarding paths that always allow this sequence to
be executed without incurring a dependency-based stall.

### 4.6 AUT* RET forwarding

In the A64 instruction set any variant of the AUT instruction will be dual issued with the directly
following RET instruction. The latency of the AUT instruction for the dependency of the LR
does not apply for these cases.

### 4.7 SIMD MAC forwarding

For the following integer SIMD instructions:
MUL, MLA, MLS, UMULL, UMULL2. SMULL, SMULL2. UMLAL. UMLAL2, SMLAL, SMLAL2,
UMLSL, UMLSL2, SMLSL, SMLAL2, UDOT, SDOT
A dedicated MAC accumulator forwarding path is present. This forwarding path will be
triggered only when two consecutive instructions satisfy the following conditions:

- Both instructions read from/write to the same destination/accumulator register
- Both instructions use the same destination element size
- The instructions target the same destination register size (128-bit or 64-bit)

When this forwarding path is active, the latency between the above instructions will be 1 cycle.

### 4.8 Memory Tagging Extensions

Enabling precise tag checking can prevent Cortex-A520 Core from entering write-streaming
mode. This can reduce performance and increase power for larger writes, and memset or
memcpy-like workloads.

### 4.9 Memory routines

To achieve maximum throughput for memory copy (or similar loops), one should do the
following:

- Unroll the loop to include multiple load and store operations per iteration, minimizing
the overheads of looping
- Stores should be aligned on a 16-byte boundary wherever possible

- Loads should not cross a 32-byte boundary as they incur a penalty

Updated optimized routines are available:
https://github.com/ARM-software/optimized-
routines/tree/master/string/aarch64

**Figure 2 shows a code snippet from the inner loop of memory copy routine that copies at least**

128 bytes. The loop copies 64 bytes per iteration and prefetches one iteration ahead.
L(loop64_simd):

```asm
str    A_q, [dst, 16]
ldr    A_q, [src, 16]
str    B_q, [dst, 32]
ldr    B_q, [src, 32]
str    C_q, [dst, 48]
ldr    C_q, [src, 48]
str    D_q, [dst, 64]!
ldr    D_q, [src, 64]!
subs    count, count, 64
b.hi    L(loop64_simd)
```

**Figure 2 Code Snippet from memcpy routine - large copy inner loop**

**Figure 3 shows a code snippet from the inner loop memory copy routine that copies 0 to 16**

bytes.
.p2align 4

```text
          /* Small copies: 0..16 bytes.                */
        L(copy16_simd):
          /* 8-15 bytes.         */
          cmp     count, 8
          b.lo     1f
          ldr     A_l, [src]
          ldr     A_h, [srcend, -8]
          str     A_l, [dstin]
          str     A_h, [dstend, -8]
          ret
          .p2align 4
        1:
          /* 4-7 bytes.        */
          tbz     count, 2, 1f
       ldr       A_lw, [src]
       ldr       A_hw, [srcend, -4]
       str       A_lw, [dstin]
       str       A_hw, [dstend, -4]
       ret
     ---
     bic src, src, 15
```

**Figure 3 Code Snippet from memcpy routine - small copy inner loop**

To achieve maximum throughput on memset, it is recommended that one do the following.

Unroll the loop to include multiple store operations per iteration, minimizing the overheads of
looping. Figure 4 shows code from the memset routine to set 17 to 96 bytes.
L(set_medium):

```asm
str      q0, [dstin]
```

```text
           tbnz     count, 6, L(set96)
           str      q0, [dstend, -16]
           tbz      count, 5, 1f
           str      q0, [dstin, 16]
           str      q0, [dstend, -32]
     1: ret
```

**Figure 4 Code snippet from memset routine**

To achieve maximum performance on memset to zero, it is recommended that one use DC ZVA
instead of STP. Figure 5 shows code from the memset routine to illustrate the usage of DC
ZVA.
L(zva_loop):

```asm
add      dst, dst, 64
dc       zva, dst
subs     count, count, 64
b.hi     L(zva_loop)
stp      q0, q0, [dstend, -64]
stp      q0, q0, [dstend, -32]
```

ret

**Figure 5 Code snipper from memset to zero routine**

### 4.10 Cache maintenance operations

While using set way invalidation operations on L1 cache, it is recommended that software be
written to traverse the sets in the inner loop and ways in the outer loop.

### 4.11 Cache access latencies

The latency numbers for load instructions given in Instruction characteristics section assume
the ideal case. It should be noted that more cycles will be added to these access delays
depending on which level of cache is accessed. Table 4-1 lists the latencies for the different
levels of cache.

Table 4-1: Cortex-A520 cache access latencies

```text
 Scenario                                              Cycle count
 L1 cache hit                                          2-4 cycles (2 is best case, 4 is normal case)
 L2 cache hit                                          10-12 cycles (10 is best case, 11-12 is normal
                                                       case)
```

### 4.12 Shared VPU

Cortex-A520 Core shares a VPU between all Cortex-A520 cores in a complex. The VPU is
used to execute ASIMD, FP, Neon, and SVE instructions. Instructions being executed on VPU
pipelines by one core may reduce performance of the instructions executed on the VPU by the
other core.

### 4.13 AES encryption / decryption

Cortex-A520 Core implements instruction fusion for AES instructions (see section 4.2). It is
recommended instructions pairs be interleaved in groups of three or more for the following:
AESE, AESMC, AESD, AESIMC.

```asm
AESE   data0, key_reg
AESMC data0, data0
AESE   data1, key_reg
AESMC data1, data1
AESE   data2, key_reg
AESMC data2, data2…
```

**Figure 6 Code snippet for AES instruction fusion**
