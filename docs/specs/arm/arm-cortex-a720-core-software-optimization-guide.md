# Arm Cortex-A720 Core Software Optimization Guide

Source title: Arm® Cortex®-A720 Core Software Optimization Guide.
Document: `109720`; metadata version: `0002`; metadata version label: `r0p2`; revision: `00`.
Cover: core revision `r0p2`; issue `7.0`.
Published: `2023-11-30`; updated: `2024-12-12`; product quality: `REL`.
Source PDF: `docs/arm_cortex_a720_core_software_optimization_guide.pdf`; SHA-256: `22a00d8f6d53ffcbc3d52074c7fe30c4b6e6512581e5e755d5804e9a0ee7bd63`.

## 1 Introduction

### 1.1 Product revision status

The rxpy identifier indicates the revision status of the product described in this book, for example,
r1p2, where:
rx
Identifies the major revision of the product, for example, r1.
py
Identifies the minor revision or modification status of the product, for example, p2.

### 1.2 Intended audience

This document is for system designers, system integrators, and programmers who are designing or
programming a System-on-Chip (SoC) that uses an Arm core.

### 1.3 Scope

This document describes aspects of the Cortex-A720 core micro-architecture that influence
software performance. Micro-architectural detail is limited to that which is useful for software
optimization.

Documentation extends only to software visible behavior of the Cortex-A720 core and not to the
hardware rationale behind the behavior.

### 1.4 Conventions

The following subsections describe conventions used in Arm documents.

#### 1.4.1 Glossary

The Arm Glossary is a list of terms used in Arm documentation, together with definitions for those
terms. The Arm Glossary does not contain terms that are industry standard unless the Arm meaning
differs from the generally accepted meaning.

See the Arm Glossary for more information: https://developer.arm.com/glossary.

#### 1.4.2 Terms and abbreviations

This document uses the following terms and abbreviations.

```text
 Term                               Meaning
 ALU                                Arithmetic and Logical Unit
 ASIMD                              Advanced SIMD
 MOP                                Macro-OPeration
 µOP                                Micro-OPeration
 SQRT                               Square Root
 FP                                 Floating-point
```

#### 1.4.3 Typographical conventions

```text
Convention          Use
italic              Introduces citations.
bold                Highlights interface elements, such as menu names. Denotes signal names. Also used for
                    terms in descriptive lists, where appropriate.
monospace           Denotes text that you can enter at the keyboard, such as commands, file and program
                    names, and source code.
monospace bold      Denotes language keywords when used outside example code.
monospace           Denotes a permitted abbreviation for a command or option. You can enter the underlined
underline           text instead of the full command or option name.
<and>               Encloses replaceable terms for assembler syntax where they appear in code or code
                    fragments.
                    For example:
                     MRC p15, 0, <Rd>, <CRn>, <CRm>, <Opcode_2>

SMALL CAPITALS      Used in body text for a few terms that have specific technical meanings, that are defined in
                    the Arm® Glossary. For example, IMPLEMENTATION DEFINED, IMPLEMENTATION SPECIFIC,
                    UNKNOWN, and UNPREDICTABLE.

                    This represents a recommendation which, if not followed, might lead to system failure or
                    damage.

                    This represents a requirement for the system that, if not followed, might result in system
                    failure or damage.

                    This represents a requirement for the system that, if not followed, will result in system
                    failure or damage.
```

This represents an important piece of information that needs your attention.

This represents a useful tip that might make it easier, better or faster to perform a task.

This is a reminder of something important that relates to the information you are reading.

### 1.5 Additional reading

This document contains information that is specific to this product. See the following documents for
other relevant information:

Table 1-1 Arm publications

```text
Document name                                           Document ID               Licensee only
Arm® Architecture Reference Manual for A-profile        DDI 0487                  No
architecture
Arm® Cortex®-A720 Core Technical Reference Manual       102530                    No
```

### 1.6 Feedback

Arm welcomes feedback on this product and its documentation.

#### 1.6.1 Feedback on this product

If you have any comments or suggestions about this product, contact your supplier and give:
- The product name.
- The product revision or version.
- An explanation with as much information as you can provide. Include symptoms and diagnostic
procedures if appropriate.

#### 1.6.2 Feedback on content

If you have comments on content, send an email to errata@arm.com and give:
- The title Arm® Cortex®-A720 Core Software Optimization Guide.
- The number 109720.
- If applicable, the page number(s) to which your comments refer.
- A concise explanation of your comments.

Arm also welcomes general suggestions for additions and improvements.

Arm tests the PDF only in Adobe Acrobat and Acrobat Reader and cannot guarantee the quality of
the represented document when used with any other PDF reader.

## 2 Overview

The Cortex-A720 core is a balanced-performance, low-power, and constrained area product that
implements the Armv9.2-A architecture. The Armv9.2-A architecture extends the architecture
defined in the Arm®v8-A architectures up to Arm®v8.7-A. It targets large screen compute
applications as well as smartphone applications.

The key features of Cortex-A720 core are:
- Implementation of the Armv9.2-A A64 instruction sets.
- AArch64 Execution state at all Exception levels, EL0 to EL3
- Memory Management Unit (MMU)
- 40-bit Physical Address (PA) and 48-bit Virtual Address (VA)
- Generic Interrupt Controller (GIC) CPU interface to connect to an external interrupt distributor
- Generic Timers interface that supports 64-bit count input from an external system counter
- Implementation of the Reliability, Availability, and Serviceability (RAS) Extension
- Implementation of the Scalable Vector Extension (SVE) with a 128-bit vector length and Scalable
Vector Extension 2 (SVE2)
- Integrated execution unit with Advanced Single Instruction Multiple Data (SIMD) and floating
point support
- Support for the optional Cryptographic Extension, which is licensed separately
- Activity Monitoring Unit (AMU)
- Separate L1 data and instruction caches
- Private, unified data and instruction L2 cache
- Optional error protection with parity or Error Correcting Code (ECC) allowing:
- Single Error Correction and Double Error Detection (SECDED) on L1 data cache and L2 cache,
and MMU Translation Cache
- Single Error Detection (SED) on L1 instruction cache and L2 Translation Lookaside Buﬀer (TLB)
- Support for Memory System Resource Partitioning and Monitoring (MPAM)
Debug features
- Armv9.2-A debug logic
- Performance Monitoring Unit (PMU)
- Embedded Trace Extension (ETE)
- Trace Buffer Extension (TRBE)
- Optional implementation of the Statistical Profiling Extension (SPE)
- Optional Embedded Logic Analyzer (ELA), ELA-600
This document describes elements of the Cortex-A720 core micro-architecture that influence
software performance so that software and compilers can be optimized accordingly.

### 2.1 Pipeline overview

The following figure describes the high-level Cortex-A720 instruction processing pipeline.

Instructions are first fetched and then decoded into internal Macro-OPerations (MOPs).

From there, the MOPs proceed through register renaming and dispatch stages.

A MOP can be split into two Micro-OPerations (µOPs) further down the pipeline after the decode
stage. Once dispatched, µOPs wait for their operands and issue out-of-order to one of thirteen issue
pipelines.

Each issue pipeline can accept one µOP per cycle.

Figure 2-1 Cortex-A720 core pipeline

Branch 0

Branch 1

Integer Single-Cycle 0

```text
                        Decode,                                 Integer Single-Cycle 1
                        Rename,
       Fetch            Dispatch
                                                            Integer Single /Multi-Cycle 0

                                                            Integer Single /Multi-Cycle 1

                                                                     FP/ASIMD 0/Vector Store data 0
                                             Issue

                                                                     FP/ASIMD 1/Vector Store data 1

                                                                 Load/Store 0

                                                                 Load/Store 1

                                                                    Load 2

                                                             Integer Store data 0

                                                             Integer Store data 1

                IN ORDER                                                OUT OF ORDER
The execution pipelines support different types of operations, as shown in the following table.

Table 2-1 Cortex-A720 core operations
  Instruction                  Instructions
  groups
Branch 0/1                   Branch µOPs
Integer Single-Cycle 0/1     Integer ALU µOPs
Integer Single/Multi-cycle Integer shift-ALU, multiply, divide and CRC µOPs
0/1
Load/Store 0/1               Load, Store address generation and special memory µOPs
Load 2                       Load µOPs
Integer Store data 0/1       Integer Store data µOPs
FP/ASIMD-0/Vector            ASIMD ALU, ASIMD misc, ASIMD integer multiply, FP convert, FP misc, FP add, FP multiply,
Store data 0                 FP divide, FP sqrt, AES µOps, crypto µOPs, store data µOPs
FP/ASIMD-1/Vector            ASIMD ALU, ASIMD misc, FP misc, FP add, FP multiply, ASIMD shift µOPs, ASIMD reduction
Store data 1                 µOPs, AES µOPs., store data µOPs
```

## 3 Instruction characteristics

### 3.1 Instruction tables

This chapter describes high-level performance characteristics for most Armv9.2-A instructions. A
series of tables summarize the effective execution latency and throughput (instruction bandwidth per
cycle), pipelines utilized, and special behaviors associated with each group of instructions. Utilized
pipelines correspond to the execution pipelines described in chapter 2.

In the tables below, Exec Latency is defined as the minimum latency seen by an operation dependent
on an instruction in the described group.

In the tables below, Execution Throughput is defined as the maximum throughput (in instructions per
cycle) of the specified instruction group that can be achieved in the entirety of the Cortex-A720 core
microarchitecture.

### 3.2 Legend for reading the utilized pipelines

Table 3-1 Cortex-A720 core pipeline names and symbols

```text
Pipeline name                                                                Symbol used in tables
Branch 0/1                                                                   B
Integer single Cycle 0/1                                                     S
Integer single Cycle 0/1 and single/multicycle 0/1                           I
Integer single/multicycle 0/1                                                M
Integer multicycle 0                                                         M0
Load/Store 01                                                                L01
Load/Store 0/1 and Load 2                                                    L
Integer Store data 0/1                                                       ID
FP/ASIMD/Vector Store data 0/1                                               V
FP/ASIMD/Vector Store data 0                                                 V0
FP/ASIMD/Vector Store data 1                                                 V1
```

### 3.3 Branch instructions

Table 3-2 AArch64 Branch instructions

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
Branch, immed                      B                   1              2                   B                   -
Branch, register                   BR, RET             1              2                   B                   -
Branch and link, immed             BL                  1              2                   B, S                -
Branch and link, register          BLR                 1              2                   B, S                -
Compare and branch                 CBZ, CBNZ, TBZ, 1                  2                   B                   -
                                   TBNZ
```

### 3.4 Arithmetic and logical instructions

Table 3-3 AArch64 Arithmetic and logical instructions

```text
Instruction Group                      AArch64             Exec           Execution           Utilized            Notes
                                       Instructions        Latency        Throughput          Pipelines
ALU, basic                             ADD, ADC, AND,      1              4                   I                   -
                                       BIC, EON, EOR,
                                       ORN, ORR, SUB,
                                       SBC
ALU, basic, flagset                    ADDS, ADCS,         1              4                   I                   -
                                       ANDS, BICS,
                                       SUBS, SBCS
ALU, extend and shift                  ADD{S}, SUB{S}      2              2                   M                   -
Arithmetic, LSL shift, shift <= 4      ADD, SUB            1              4                   I                   -
Arithmetic, flagset, LSL shift,        ADDS, SUBS          1              4                   I                   -
shift <= 4
Arithmetic, LSR/ASR/ROR shift          ADD{S}, SUB{S}      2              2                   M                   -
or LSL shift > 4
Arithmetic, immediate to logical       ADDG, SUBG          1              4                   I                   -
address tag
Conditional compare                    CCMN, CCMP          1              4                   I                   -
Conditional select                     CSEL, CSINC,        1              4                   I                   -
                                       CSINV, CSNEG
Convert floating-point condition AXFLAG, XAFLAG 1                         4                   I                   -
flags
Flag manipulation instructions         SETF8, SETF16,      1              4                   I                   -
                                       RMIF, CFINV
Insert Random Tags                     IRG                 2              1                   M0                   1
Insert Tag Mask                        GMI                 1              4                   I                   -
Logical, shift, no flagset             AND, BIC, EON,      1              4                   I                   -
                                       EOR, ORN, ORR
Logical, shift, flagset                ANDS, BICS          2              2                   M                   -
Subtract Pointer                       SUBP                1              4                   I                   -
Subtract Pointer, flagset              SUBPS               1              3                   I                   -
Notes:
1.The latency is 2, throughput is 1 and utilized pipeline is M0 when GCR_EL1.RRND = 1. When GCR_EL1.RRND = 0, the
description is not valid, execution throughput and latency are degradated.
```

### 3.5 Divide and multiply instructions

Table 3-4 AArch64 Divide and multiply instructions

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
Divide, W-form                     SDIV, UDIV          5 to 12        1/12 to 1/5         M0                  1
Divide, X-form                     SDIV, UDIV          5 to 20        1/20 to 1/5         M0                  1
Multiply accumulate, W-form        MADD, MSUB          2(1)           1                   M0                  2, 3
Multiply accumulate, X-form        MADD, MSUB          2(1)           1                   M0                  2, 3
Multiply accumulate long           SMADDL,             2(1)           1                   M0                  2, 3
                                   SMSUBL,
                                   UMADDL,
                                   UMSUBL
Multiply high                      SMULH, UMULH        3              2                   M                   2
Notes:
1. Integer divides are performed using an iterative algorithm and block any subsequent divide operations until complete.
Early termination is possible, depending upon the data values.
2. Multiply-accumulate pipelines support late-forwarding of accumulate operands from similar µOPs, allowing a typical
sequence of multiply-accumulate µOPs to issue one every N cycles (accumulate latency N shown in parentheses).
Accumulator forwarding is not supported for consumers of 64 bit multiply high operations.
3. Multiply without accumulate when Ra is ZR (0'b11111), MUL, MNEG, SMULL, SMNEGL, UMULL and UMNEGL
instructions can be executed on utilized pipeline M with an execution throughput of 2.
```

### 3.6 Pointer Authentication Instructions

Table 3-5 AArch64 pointer authentication instructions

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
Authenticate data address          AUTDA, AUTDB,       1              2                   M
                                   AUTDZA,
                                   AUTDZB
Authenticate instruction address AUTIA, AUTIB,   1                    2                   M
                                 AUTIA1716,
                                 AUTIB1716,
                                 AUTIASP,
                                 AUTIBSP,
                                 AUTIAZ, AUTIBZ,
                                 AUTIZA, AUTIZB
Branch and link, register, with    BLRAA, BLRAAZ,      2              2                   M, B                1
pointer authentication             BLRAB, BLRABZ
Branch, register, with pointer     BRAA, BRAAZ,        2              2                   M, B                1
authentication                     BRAB, BRABZ
Branch, return, with pointer       RETA, RETB          2              2                   M, B                1
authentication
Compute pointer authentication PACDA, PACDB,           4              2                   M
code for data address          PACDZA,
                               PACDZB
Compute pointer authentication PACGA                   4              2                   M
code, using generic key
Compute pointer authentication PACIA, PACIB,   4                      2                   M
code for instruction address   PACIA1716,
                               PACIB1716,
                               PACIASP,
                               PACIBSP,
                               PACIAZ, PACIBZ,
                               PACIZA, PACIZB
Load register, with pointer        LDRAA, LDRAB        5              2                   M, L, I             1, 2
authentication
Strip pointer authentication       XPACD, XPACI,       1              2                   M
code                               XPACLRI
Notes:
1.In case of AUTH FAIL the description is not valid, execution throughput and latency are degraded.
2. Only Immed pre-index with write back use I pipes
```

### 3.7 Miscellaneous data-processing instructions

Table 3-6 AArch64 Miscellaneous data-processing instructions

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
Address generation                 ADR, ADRP           1              2                   S                   -
Bitfield extract, one, two regs    EXTR                1              4                   I                   -
Bitfield move, basic               SBFM, UBFM          1              4                   I                   -
Bitfield move, insert              BFM                 1              4                   I                   -
Count leading                      CLS, CLZ            1              4                   I                   -
Move immed                         MOVN, MOVK,         1              4                   I                   -
                                   MOVZ
Reverse bits/bytes                 RBIT, REV,          1              4                   I                   -
                                   REV16, REV32
Variable shift                     ASRV, LSLV,         1              4                   I                   -
                                   LSRV, RORV
```

### 3.8 Load instructions

The latencies shown assume the memory access hits in the Level 1 Data Cache and represent the maximum latency to load
all the registers written by the instruction.

Table 3-7 AArch64 Load instructions

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
Load register, literal             LDR, LDRSW,         5              2                   L, S                -
                                   PRFM
Load register, unscaled immed      LDUR, LDURB,   4                   3                   L                   -
                                   LDURH, LDURSB,
                                   LDURSH,
                                   LDURSW,
                                   PRFUM
Load register, immed post-index LDR, LDRB,             4              3                   L, I                -
                                LDRH, LDRSB,
                                LDRSH, LDRSW
Load register, immed pre-index     LDR, LDRB,          4              3                   L, I                1
                                   LDRH, LDRSB,
                                   LDRSH, LDRSW
Load register, immed               LDTR, LDTRB,        4              3                   L                   -
unprivileged                       LDTRH, LDTRSB,
                                   LDTRSH,
                                   LDTRSW
Load register, unsigned immed      LDR, LDRB,          4              3                   L                   -
                                   LDRH, LDRSB,
                                   LDRSH, LDRSW,
                                   PRFM
Instruction Group                   AArch64               Exec           Execution            Utilized         Notes
                                    Instructions          Latency        Throughput           Pipelines
Load register, register offset,     LDR, LDRB,            4              3                    L                2
basic                               LDRH, LDRSB,
                                    LDRSH, LDRSW,
                                    PRFM
Load register, register offset,     LDR, LDRSW,           4              3                    L                2
scale by 4/8                        PRFM
Load register, register offset,     LDRH, LDRSH           4              3                    L                2
scale by 2
Load register, register offset,     LDR, LDRB,            4              3                    L                2
extend                              LDRH, LDRSB,
                                    LDRSH, LDRSW,
                                    PRFM
Load register, register offset,     LDR, LDRSW,           4              3                    L                2
extend, scale by 4/8                PRFM
Load register, register offset,     LDRH, LDRSH           4              3                    L                2
extend, scale by 2
Load pair, signed immed offset,     LDP, LDNP             4              3                    L                -
normal, W-form
Load pair, signed immed offset,     LDP, LDNP             4              3/2                  L                -
normal, X-form
Load pair, signed immed offset,     LDPSW                 4              3/2                  I, L             -
signed words
Load pair, immed post-index or      LDP                   4              3                    L, I             -
immed pre-index, normal, W-
form
Load pair, immed post-index or  LDP                       4              3/2                  L, I             -
immed pre-index, normal, X-form
Load pair, immed post-index or      LDPSW                 4              3/2                  I, L             -
immed pre-index, signed words
Notes:
1. Only Immed pre-index with write back use I pipes
2. Execution Latency is 5 and Utilized Pipelines are L, I when scale with aligned offset of 128 bits
```

### 3.9 Store instructions

The following table describes performance characteristics for standard store instructions. Stores
µOPs are split into address and data µOPs. Once executed, stores are buffered and committed in the
background.

Table 3-8 AArch64 Store instructions

```text
Instruction Group                    AArch64             Exec           Execution           Utilized            Notes
                                     Instructions        Latency        Throughput          Pipelines
Store register, unscaled immed       STUR, STURB,        1              2                   L01, ID             -
                                     STURH
Store register, immed post-index STR, STRB, STRH         1              2                   L01, ID, I          -
Store register, immed pre-index      STR, STRB, STRH     1              2                   L01, ID, I          -
Store register, immed                STTR, STTRB,        1              2                   L01, ID             -
unprivileged                         STTRH
Store register, unsigned immed       STR, STRB, STRH     1              2                   L01, ID             -
Store register, register offset,     STR, STRB, STRH     1              2                   L01, ID             -
basic
Store register, register offset,     STR                 1              2                   L01, ID             -
scaled by 4/8
Store register, register offset,     STRH                1              2                   L01, ID             -
scaled by 2
Store register, register offset,     STR, STRB, STRH     1              2                   L01, ID             -
extend
Store register, register offset,     STR                 1              2                   L01, ID             -
extend, scale by 4/8
Store register, register offset,     STRH                1              2                   L01, ID             -
extend, scale by 2
Store pair, immed offset             STP, STNP           1              2                   L01, ID             -
Store pair, immed post-index         STP                 1              2                   L01, ID, I          -
Store pair, immed pre-index          STP                 1              2                   L01, ID, I          -
```

### 3.10 Tag Load Instructions

Table 3-9 AArch64 Tag load instructions

```text
Instruction Group                    AArch64             Exec           Execution           Utilized            Notes
                                     Instructions        Latency        Throughput          Pipelines
Load allocation tag                  LDG                 5              3                   L, I                -
Load multiple allocation tags        LDGM                4              3                   L                   -
```

### 3.11 Tag Store instructions

Table 3-10 AArch64 Tag store instructions

```text
Instruction Group                   AArch64            Exec           Execution           Utilized            Notes
                                    Instructions       Latency        Throughput          Pipelines

Store allocation tags to one or     STG, ST2G          1              2                   L01, ID, I          -
two granules, post-index

Store allocation tags to one or     STG, ST2G          1              2                   L01, ID, I          -
two granules, pre-index

Store allocation tags to one or     STG, ST2G          1              2                   L01, ID             -
two granules, signed offset

Store allocation tag to one or      STZG, STZ2G        1              2                   L01, ID, I          -
two granules, zeroing, post-
index

Store Allocation Tag to one or      STZG, STZ2G        1              2                   L01, ID, I          -
two granules, zeroing, pre-index

Store allocation tag to two         STZG, STZ2G        1              2                   L01, ID             -
granules, zeroing, signed offset

Store allocation tag and reg pair   STGP               1              2                   L01, ID, I          -
to memory, post-Index

Store allocation tag and reg pair   STGP               1              2                   L01, ID, I          -
to memory, pre-Index

Store allocation tag and reg pair   STGP               1              2                   L01, ID             -
to memory, signed offset

Store multiple allocation tags      STGM               1              2                   L01, ID             -

Store multiple allocation tags,     STZGM              1              2                   L01, ID             -
zeroing
```

### 3.12 FP data processing instructions

Table 3-11 AArch64 FP data processing instructions

```text
Instruction Group                   AArch64            Exec           Execution           Utilized            Notes
                                    Instructions       Latency        Throughput          Pipelines
FP absolute value                   FABS, FABD         2              2                   V                   -
FP arithmetic                       FADD, FSUB         2              2                   V                   -
FP compare                          FCCMP{E},          2              2                   V                   -
                                    FCMP{E}
FP divide, H-form                   FDIV               5              1                   V0                  1
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
FP divide, S-form                  FDIV                7              1                   V0                  1
FP divide, D-form                  FDIV                12             1                   V0                  1
FP min/max                         FMIN, FMINNM,       2              2                   V                   -
                                   FMAX, FMAXNM
FP multiply                        FMUL, FNMUL         3              2                   V                   2
FP multiply accumulate             FMADD, FMSUB, 4 (2)                2                   V                   3
                                   FNMADD,
                                   FNMSUB
FP negate                          FNEG                2              2                   V                   -
FP round to integral               FRINTA, FRINTI, 3                  1                   V0                  -
                                   FRINTM,
                                   FRINTN, FRINTP,
                                   FRINTX, FRINTZ,
                                   FRINT32X,
                                   FRINT64X,
                                   FRINT32Z,
                                   FRINT64Z
FP select                          FCSEL               2              2                   V                   -
FP square root, H-form             FSQRT               5              1                   V0                  1
FP square root, S-form             FSQRT               7              1                   V0                  1
FP square root, D-form             FSQRT               12             1                   V0                  1
Notes:
1. FP divide and square root operations are now performed using a fully pipelined data path.
2. FP multiply-accumulate pipelines support late forwarding of the result from FP multiply µOPs to the accumulate operands
of an FP multiply-accumulate µOP. The latter can potentially be issued 1 cycle after the FP multiply µOP has been issued.
3. FP multiply-accumulate pipelines support late-forwarding of accumulate operands from similar µOPs, allowing a typical
sequence of multiply-accumulate µOPs to issue one every N cycles (accumulate latency N shown in parentheses).
```

### 3.13 FP miscellaneous instructions

Table 3-12 AArch64 FP miscellaneous instructions

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
FP convert, from gen to vec reg    SCVTF, UCVTF        3              1                   M0                  -
FP convert, from vec to gen reg    FCVTAS,             3              1                   V0                  -
                                   FCVTAU,
                                   FCVTMS,
                                   FCVTMU,
                                   FCVTNS,
                                   FCVTNU,
                                   FCVTPS,
                                   FCVTPU,
                                   FCVTZS,
                                   FCVTZU
Instruction Group                   AArch64            Exec           Execution           Utilized            Notes
                                    Instructions       Latency        Throughput          Pipelines
FP convert, Javascript from vec     FJCVTZS            3              1                   V0                  -
to gen reg
FP convert, from vec to vec reg     FCVT, FCVTXN       3              1                   V0                  -
FP move, immed                      FMOV               2              2                   V                   1
FP move, register                   FMOV               2              2                   V                   1
FP transfer, from gen to low half FMOV                 3              1                   M0                  -
of vec reg
FP transfer, from gen to high half FMOV                5              1                   M0, V               -
of vec reg
FP transfer, from vec to gen reg    FMOV               3              2                   V                   -
Notes:
1. Particular FMOV #0 or Register to Register can be optimized in rename stage pipeline, execution latency and throughput
are then not representative.
```

### 3.14 FP load instructions

The latencies shown assume the memory access hits in the Level 1 Data Cache and represent the
maximum latency to load all the vector registers written by the instruction. Compared to standard
loads, an extra cycle is required to forward results to FP/ASIMD pipelines.

Table 3-13 AArch64 FP load instructions

```text
Instruction Group                   AArch64            Exec           Execution           Utilized            Notes
                                    Instructions       Latency        Throughput          Pipelines
Load vector reg, literal, S/D/Q     LDR                6              3                   L                   -
forms
Load vector reg, unscaled immed LDUR                   6              3                   L                   -
Load vector reg, immed post-        LDR                6              3                   L, I                -
index
Load vector reg, immed pre-         LDR                6              3                   L, I                -
index
Load vector reg, unsigned           LDR                6              3                   L                   -
immed
Load vector reg, register offset,   LDR                6              3                   L                   -
basic
Load vector reg, register offset,   LDR                6              3                   L                   -
scale, S/D-form
Load vector reg, register offset,   LDR                6              3                   L                   -
scale, H/Q-form
Load vector reg, register offset,   LDR                6              3                   L                   -
extend
Load vector reg, register offset,   LDR                6              3                   L                   -
extend, scale, S/D-form
Instruction Group                    AArch64           Exec           Execution           Utilized            Notes
                                     Instructions      Latency        Throughput          Pipelines
Load vector reg, register offset,    LDR               6              3                   L                   -
extend, scale, H/Q-form
Load vector pair, immed offset,      LDP, LDNP         6              3                   L                   -
S/D-form
Load vector pair, immed offset,      LDP, LDNP         6              3/2                 L                   -
Q-form
Load vector pair, immed post-        LDP               6              3/2                 I, L                -
index, S/D-form
Load vector pair, immed post-        LDP               6              3/2                 L, I                -
index, Q-form
Load vector pair, immed pre-         LDP               6              3/2                 I, L                -
index, S/D-form
Load vector pair, immed pre-         LDP               6              3/2                 L, I                -
index, Q-form
```

### 3.15 FP store instructions

Stores MOPs are split into store address and store data µOPs. Once executed, stores are buffered
and committed in the background.

Table 3-14 AArch64 FP store instructions

```text
Instruction Group                    AArch64           Exec           Execution           Utilized            Notes
                                     Instructions      Latency        Throughput          Pipelines
Store vector reg, unscaled           STUR              2              2                   L01, V              -
immed, B/H/S/D-form
Store vector reg, unscaled           STUR              2              2                   L01, V              -
immed, Q-form
Store vector reg, immed post-        STR               2              2                   L01, V, I           -
index, B/H/S/D-form
Store vector reg, immed post-        STR               2              2                   L01, V, I           -
index, Q-form
Store vector reg, immed pre-         STR               3              2                   L01, V, I           -
index, B/H/S/D-form
Store vector reg, immed pre-         STR               2              2                   L01, V, I           -
index, Q-form
Store vector reg, unsigned           STR               2              2                   L01, V              -
immed, B/H/S/D-form
Store vector reg, unsigned           STR               2              2                   L01, V              -
immed, Q-form
Store vector reg, register offset,   STR               2              2                   L01, V              -
basic, B/H/S/D-form
Store vector reg, register offset,   STR               2              2                   L01, V              -
basic, Q-form
Instruction Group                    AArch64           Exec           Execution           Utilized            Notes
                                     Instructions      Latency        Throughput          Pipelines
Store vector reg, register offset,   STR               2              2                   L01, V              -
scale, H-form
Store vector reg, register offset,   STR               2              2                   L01, V              -
scale, S/D-form
Store vector reg, register offset,   STR               2              2                   I, L01, V           -
scale, Q-form
Store vector reg, register offset,   STR               2              2                   L01, V              -
extend, B/H/S/D-form
Store vector reg, register offset,   STR               2              2                   L01, V              -
extend, Q-form
Store vector reg, register offset,   STR               2              2                   L01, V              -
extend, scale, H-form
Store vector reg, register offset,   STR               2              2                   L01, V              -
extend, scale, S/D-form
Store vector reg, register offset,   STR               2              2                   I, L01, V           -
extend, scale, Q-form
Store vector pair, immed offset,     STP, STNP         2              2                   L01, V              -
S-form
Store vector pair, immed offset,     STP, STNP         2              2                   L01, V              -
D-form
Store vector pair, immed offset,     STP, STNP         2              2                   L01, V              -
Q-form
Store vector pair, immed post-       STP               2              2                   I, L01, V           -
index, S-form
Store vector pair, immed post-       STP               2              2                   I, L01, V           -
index, D-form
Store vector pair, immed post-       STP               2              2                   I, L01, V           -
index, Q-form
Store vector pair, immed pre-        STP               2              2                   I, L01, V           -
index, S-form
Store vector pair, immed pre-        STP               2              2                   I, L01, V           -
index, D-form
Store vector pair, immed pre-        STP               2              2                   I, L01, V           -
index, Q-form
```

### 3.16 ASIMD integer instructions

Table 3-15 AArch64 ASIMD integer instructions

```text
Instruction Group                    AArch64           Exec           Execution           Utilized            Notes
                                     Instructions      Latency        Throughput          Pipelines
ASIMD absolute diff                  SABD, UABD        2              2                   V                   -
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
ASIMD absolute diff accum          SABA, UABA          4(1)           1                   V1                  2
ASIMD absolute diff accum long     SABAL(2),           4(1)           1                   V1                  2
                                   UABAL(2)
ASIMD absolute diff long           SABDL(2),           2              2                   V                   -
                                   UABDL(2)
ASIMD arith, basic                 ABS, ADD, NEG, 2                   2                   V                   -
                                   SADDL(2),
                                   SADDW(2),
                                   SHADD, SHSUB,
                                   SSUBL(2),
                                   SSUBW(2), SUB,
                                   UADDL(2),
                                   UADDW(2),
                                   UHADD, UHSUB,
                                   USUBL(2),
                                   USUBW(2)
ASIMD arith, complex               ADDHN(2),     2                    2                   V                   -
                                   RADDHN(2),
                                   RSUBHN(2),
                                   SQABS, SQADD,
                                   SQNEG, SQSUB,
                                   SRHADD,
                                   SUBHN(2),
                                   SUQADD,
                                   UQADD, UQSUB,
                                   URHADD,
                                   USQADD
ASIMD arith, pair-wise             ADDP, SADDLP,       2              2                   V                   -
                                   UADDLP
ASIMD arith, reduce, 4H/4S         ADDV, SADDLV,       3              1                   V1                  -
                                   UADDLV
ASIMD arith, reduce, 8B/8H         ADDV, SADDLV,       5              1                   V1, V               -
                                   UADDLV
ASIMD arith, reduce, 16B           ADDV, SADDLV,       6              1/2                 V1                  -
                                   UADDLV
ASIMD compare                      CMEQ, CMGE,         2              2                   V                   -
                                   CMGT, CMHI,
                                   CMHS, CMLE,
                                   CMLT, CMTST
ASIMD dot product                  SDOT, UDOT          3 (1)          2                   V                   2
ASIMD dot product using signed SUDOT, USDOT            3(1)           2                   V                   2
and unsigned integers
ASIMD logical                      AND, BIC, EOR, 2                   2                   V                   -
                                   MOV, MVN, NOT,
                                   ORN, ORR
ASIMD matrix multiply-             SMMLA, UMMLA, 3(1)                 2                   V                   2
accumulate                         USMMLA
Instruction Group                AArch64            Exec           Execution           Utilized            Notes
                                 Instructions       Latency        Throughput          Pipelines
ASIMD max/min, basic and pair-   SMAX, SMAXP,       2              2                   V                   -
wise                             SMIN, SMINP,
                                 UMAX, UMAXP,
                                 UMIN, UMINP
ASIMD max/min, reduce, 4H/4S     SMAXV, SMINV,      3              1                   V1                  -
                                 UMAXV, UMINV
ASIMD max/min, reduce, 8B/8H SMAXV, SMINV,          5              1                   V1, V               -
                             UMAXV, UMINV
ASIMD max/min, reduce, 16B       SMAXV, SMINV,      6              1/2                 V1                  -
                                 UMAXV, UMINV
ASIMD multiply                   MUL, SQDMULH, 4                   1                   V0                  -
                                 SQRDMULH
ASIMD multiply accumulate        MLA, MLS           4(1)           1                   V0                  1
ASIMD multiply accumulate high SQRDMLAH,            4(2)           1                   V0                  1
                               SQRDMLSH
ASIMD multiply accumulate long SMLAL(2),            4(1)           1                   V0                  1
                               SMLSL(2),
                               UMLAL(2),
                               UMLSL(2)
ASIMD multiply accumulate        SQDMLAL(2),        4(2)           1                   V0                  1
saturating long                  SQDMLSL(2)
ASIMD multiply/multiply long     PMUL, PMULL(2) 2                  1                   V0                  3
(8x8) polynomial, D-form
ASIMD multiply/multiply long     PMUL, PMULL(2) 2                  1                   V0                  3
(8x8) polynomial, Q-form
ASIMD multiply long              SMULL(2),          4              1                   V0                  -
                                 UMULL(2),
                                 SQDMULL(2)
ASIMD pairwise add and           SADALP,            4(1)           1                   V1                  2
accumulate long                  UADALP
ASIMD shift accumulate           SSRA, SRSRA,       4(1)           1                   V1                  2
                                 USRA, URSRA
ASIMD shift by immed, basic      SHL, SHLL(2),      2              1                   V1                  -
                                 SHRN(2),
                                 SSHLL(2), SSHR,
                                 SXTL(2),
                                 USHLL(2), USHR,
                                 UXTL(2)
ASIMD shift by immed and         SLI, SRI           2              1                   V1                  -
insert, basic
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
ASIMD shift by immed, complex      RSHRN(2),           4              1                   V1                  -
                                   SQRSHRN(2),
                                   SQRSHRUN(2),
                                   SQSHL{U},
                                   SQSHRN(2),
                                   SQSHRUN(2),
                                   SRSHR,
                                   UQRSHRN(2),
                                   UQSHL,
                                   UQSHRN(2),
                                   URSHR
ASIMD shift by register, basic     SSHL, USHL          2              1                   V1                  -
ASIMD shift by register, complex SRSHL, SQRSHL, 4                     1                   V1                  -
                                 SQSHL, URSHL,
                                 UQRSHL, UQSHL
Notes:
1. Multiply-accumulate pipelines support late-forwarding of accumulate operands from similar µOPs, allowing a typical
sequence of integer multiply-accumulate µOPs to issue one every cycle or one every other cycle (accumulate latency shown
in parentheses).
2. Other accumulate pipelines also support late-forwarding of accumulate operands from similar µOPs, allowing a typical
sequence of such µOPs to issue one every cycle (accumulate latency shown in parentheses).
3. This category includes instructions of the form “PMULL Vd.8H, Vn.8B, Vm.8B” and “PMULL2 Vd.8H, Vn.16B, Vm.16B”.
```

### 3.17 ASIMD floating-point instructions

Table 3-16 AArch64 ASIMD floating-point instructions

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
ASIMD FP absolute                  FABS, FABD          2              2                   V                   -
value/difference
ASIMD FP arith, normal             FADD, FSUB          2              2                   V                   -
ASIMD FP compare                   FACGE, FACGT, 2                    2                   V                   -
                                   FCMEQ, FCMGE,
                                   FCMGT, FCMLE,
                                   FCMLT
ASIMD FP complex add               FCADD               3              2                   V                   -
ASIMD FP complex multiply add FCMLA                    4(2)           2                   V                   1
ASIMD FP convert, long (F16 to     FCVTL(2)            4              1/2                 V0                  -
F32)
ASIMD FP convert, long (F32 to     FCVTL(2)            3              1                   V0                  -
F64)
ASIMD FP convert, narrow (F32 FCVTN(2)                 4              1/2                 V0                  -
to F16)
ASIMD FP convert, narrow (F64 FCVTN(2),                3              1                   V0                  -
to F32)                       FCVTXN(2)
Instruction Group               AArch64             Exec           Execution           Utilized            Notes
                                Instructions        Latency        Throughput          Pipelines
ASIMD FP convert, other, D-     FCVTAS,             3              1                   V0                  -
form F32 and Q-form F64         FCVTAU,
                                FCVTMS,
                                FCVTMU,
                                FCVTNS,
                                FCVTNU,
                                FCVTPS,
                                FCVTPU,
                                FCVTZS,
                                FCVTZU, SCVTF,
                                UCVTF
ASIMD FP convert, other, D-     FCVTAS,             4              1/2                 V0                  -
form F16 and Q-form F32         VCVTAU,
                                FCVTMS,
                                FCVTMU,
                                FCVTNS,
                                FCVTNU,
                                FCVTPS,
                                FCVTPU,
                                FCVTZS,
                                FCVTZU, SCVTF,
                                UCVTF
ASIMD FP convert, other, Q-     FCVTAS,             6              1/4                 V0                  -
form F16                        VCVTAU,
                                FCVTMS,
                                FCVTMU,
                                FCVTNS,
                                FCVTNU,
                                FCVTPS,
                                FCVTPU,
                                FCVTZS,
                                FCVTZU, SCVTF,
                                UCVTF
ASIMD FP divide, D-form, F16    FDIV                8              1/4                 V0                  3
ASIMD FP divide, D-form, F32    FDIV                8              1/2                 V0                  3
ASIMD FP divide, Q-form, F16    FDIV                12             1/8                 V0                  3
ASIMD FP divide, Q-form, F32    FDIV                10             1/4                 V0                  3
ASIMD FP divide, Q-form, F64    FDIV                13             1/2                 V0                  3
ASIMD FP max/min, normal        FMAX, FMAXNM, 2                    2                   V                   -
                                FMIN, FMINNM
ASIMD FP arith, max/min,        FADDP, FMAXP,       3              2                   V                   -
pairwise                        FMAXNMP,
                                FMINP,
                                FMINNMP
ASIMD FP max/min, reduce, F32 FMAXV,                4              1                   V                   -
and D-form F16                FMAXNMV,
                              FMINV,
                              FMINNMV
Instruction Group                 AArch64             Exec           Execution           Utilized            Notes
                                  Instructions        Latency        Throughput          Pipelines
ASIMD FP max/min, reduce, Q-      FMAXV,              6              2/3                 V                   -
form F16                          FMAXNMV,
                                  FMINV,
                                  FMINNMV
ASIMD FP multiply                 FMUL, FMULX         3              2                   V                   2
ASIMD FP multiply accumulate      FMLA, FMLS          4(2)           2                   V                   1
ASIMD FP multiply accumulate      FMLAL(2),           4(2)           2                   V                   1
long                              FMLSL(2)
ASIMD FP negate                   FNEG                2              2                   V                   -
ASIMD FP round, D-form F32        FRINTA, FRINTI, 3                  1                   V0                  -
and Q-form F64                    FRINTM,
                                  FRINTN, FRINTP,
                                  FRINTX, FRINTZ,
                                  FRINT32X,
                                  FRINT64X,
                                  FRINT32Z,
                                  FRINT64Z
ASIMD FP round, D-form F16        FRINTA, FRINTI, 4                  1/2                 V0                  -
and Q-form F32                    FRINTM,
                                  FRINTN, FRINTP,
                                  FRINTX, FRINTZ,
                                  FRINT32X,
                                  FRINT64X,
                                  FRINT32Z,
                                  FRINT64Z
ASIMD FP round, Q-form F16        FRINTA, FRINTI, 6                  1/4                 V0                  -
                                  FRINTM,
                                  FRINTN, FRINTP,
                                  FRINTX, FRINTZ,
                                  FRINT32X,
                                  FRINT64X,
                                  FRINT32Z,
                                  FRINT64Z
ASIMD FP square root, D-form,     FSQRT               8              1/4                 V0                  3
F16
ASIMD FP square root, D-form,     FSQRT               8              1/2                 V0                  3
F32
ASIMD FP square root, Q-form,     FSQRT               12             1/8                 V0                  3
F16
ASIMD FP square root, Q-form,     FSQRT               10             1/4                 V0                  3
F32
ASIMD FP square root, Q-form,     FSQRT               13             1/2                 V0                  3
F64
Notes:
1. ASIMD multiply-accumulate pipelines support late-forwarding of accumulate operands from similar µOPs, allowing a
typical sequence of floating-point multiply-accumulate µOPs to issue one every N cycles (accumulate latency N shown in
parentheses).
2. ASIMD multiply-accumulate pipelines support late forwarding of the result from ASIMD FP multiply µOPs to the
accumulate operands of an ASIMD FP multiply-accumulate µOP. The latter can potentially be issued 1 cycle after the
ASIMD FP multiply µOP has been issued.
3. ASIMD FP divide and square root operations are now performed using a fully pipelined data path.
```

### 3.18 ASIMD BFloat16 (BF16) instructions

Table 3-17 AArch64 ASIMD BFloat (BF16) instructions

```text
Instruction Group                 AArch64             Exec           Execution           Utilized            Notes
                                  Instructions        Latency        Throughput          Pipelines
ASIMD convert, F32 to BF16        BFCVTN,             4              1/2                 V0                  -
                                  BFCVTN2
ASIMD dot product                 BFDOT               4(2)           2                   V                   1
ASIMD matrix multiply             BFMMLA              5(3)           2                   V                   1
accumulate
ASIMD multiply accumulate long BFMLALB,               4(2)           2                   V                   1
                               BFMLALT
Scalar convert, F32 to BF16       BFCVT               3              1                   V0                  -
```

Notes:
1. ASIMD pipelines that execute these instructions support late-forwarding of accumulate operands from similar µOPs,
allowing a typical sequence of µOPs to issue one every N cycles (accumulate latency N shown in parentheses).

### 3.19 ASIMD miscellaneous instructions

Table 3-18 AArch64 ASIMD miscellaneous instructions

```text
Instruction Group                 AArch64             Exec           Execution           Utilized            Notes
                                  Instructions        Latency        Throughput          Pipelines
ASIMD bit reverse                 RBIT                2              2                   V                   2
ASIMD bitwise insert              BIF, BIT, BSL       2              2                   V
ASIMD count                       CLS, CLZ, CNT       2              2                   V                   -
ASIMD duplicate, gen reg          DUP                 3              1                   M0                  -
ASIMD duplicate, element          DUP                 2              2                   V                   2
ASIMD extract                     EXT                 2              2                   V                   2
ASIMD extract narrow              XTN(2)              2              2                   V
ASIMD extract narrow,             SQXTN(2),           4              1                   V1                  -
saturating                        SQXTUN(2),
                                  UQXTN(2)
ASIMD insert, element to          INS                 2              2                   V                   2
element
ASIMD move, FP immed              FMOV                2              2                   V                   1
ASIMD move, integer immed         MOVI, MVNI          2              2                   V                   -
ASIMD reciprocal and square       URECPE,             3              1                   V0                  -
root estimate, D-form U32         URSQRTE
ASIMD reciprocal and square       URECPE,             4              1/2                 V0                  -
root estimate, Q-form U32         URSQRTE
ASIMD reciprocal and square       FRECPE,             3              1                   V0                  -
root estimate, D-form F32 and     FRSQRTE
scalar forms
ASIMD reciprocal and square       FRECPE,             4              1/2                 V0                  -
root estimate, D-form F16 and     FRSQRTE
Q-form F32
ASIMD reciprocal and square       FRECPE,             6              1/4                 V0                  -
root estimate, Q-form F16         FRSQRTE
ASIMD reciprocal exponent         FRECPX              3              1                   V0
ASIMD reciprocal step             FRECPS,             4              2                   V                   -
                                  FRSQRTS
ASIMD reverse                     REV16, REV32,       2              2                   V                   2
                                  REV64
ASIMD table lookup, 1 or 2 table TBL                  2              2                   V                   2
regs
ASIMD table lookup, 3 table regs TBL                  4              1                   V                   2
ASIMD table lookup, 4 table regs TBL                  4              2/3                 V                   2
ASIMD table lookup extension, 1 TBX                   2              2                   V                   2
table reg
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
ASIMD table lookup extension, 2 TBX                    4              1                   V                   2
table reg
ASIMD table lookup extension, 3 TBX                    6              2/3                 V                   2
table reg
ASIMD table lookup extension, 4 TBX                    6              1/2                 V                   2
table reg
ASIMD transfer, element to gen     UMOV, SMOV          2              1                   V                   -
reg
ASIMD transfer, gen reg to         INS                 5              1                   M0, V
element
ASIMD transpose                    TRN1, TRN2          2              2                   V                   2
ASIMD unzip/zip                    UZP1, UZP2,         2              2                   V                   2
                                   ZIP1, ZIP2
Notes:
1. Particular FMOV #0 or Register to Register can be optimized in rename stage pipeline, execution latency and throughput
are then not representative.
2 PERM instructions part of a particular region forwarding
```

### 3.20 ASIMD load instructions

The latencies shown assume the memory access hits in the Level 1 Data Cache and represent the
maximum latency to load all the vector registers written by the instruction. Compared to standard
loads, an extra cycle is required to forward results to FP/ASIMD pipelines.

Table 3-19 AArch64 ASIMD load instructions

```text
Instruction Group               AArch64             Exec           Execution           Utilized            Notes
                                Instructions        Latency        Throughput          Pipelines
ASIMD load, 1 element, multiple, LD1                6              3                   L                   -
1 reg, D-form
ASIMD load, 1 element, multiple, LD1                6              3                   L                   -
1 reg, Q-form
ASIMD load, 1 element, multiple, LD1                6              3/2                 L                   -
2 reg, D-form
ASIMD load, 1 element, multiple, LD1                6              3/2                 L                   -
2 reg, Q-form
ASIMD load, 1 element, multiple, LD1                6              1                   L                   -
3 reg, D-form
ASIMD load, 1 element, multiple, LD1                6              1                   L                   -
3 reg, Q-form
ASIMD load, 1 element, multiple, LD1                7              3/4                 L                   -
4 reg, D-form
ASIMD load, 1 element, multiple, LD1                7              3/4                 L                   -
4 reg, Q-form
ASIMD load, 1 element, one lane, LD1                8              2                   L, V                -
B/H/S
ASIMD load, 1 element, one lane, LD1                8              2                   L, V                -
D
ASIMD load, 1 element, all lanes, LD1R              6              3                   L                   -
D-form, B/H/S
ASIMD load, 1 element, all lanes, LD1R              6              3                   L                   -
D-form, D
ASIMD load, 1 element, all lanes, LD1R              6              3                   L                   -
Q-form
ASIMD load, 2 element, multiple, LD2                8              2                   L, V                -
D-form, B/H/S
ASIMD load, 2 element, multiple, LD2                8              3/2                 L, V                -
Q-form, B/H/S
ASIMD load, 2 element, multiple, LD2                8              3/2                 L, V                -
Q-form, D
ASIMD load, 2 element, one lane, LD2                8              2                   L, V                -
B/H
ASIMD load, 2 element, one lane, LD2                8              2                   L, V                -
S
Instruction Group               AArch64             Exec           Execution           Utilized            Notes
                                Instructions        Latency        Throughput          Pipelines
ASIMD load, 2 element, one lane, LD2                8              2                   L, V                -
D
ASIMD load, 2 element, all lanes, LD2R              6              3/2                 L                   -
D-form, B/H/S
ASIMD load, 2 element, all lanes, LD2R              6              3/2                 L                   -
D-form, D
ASIMD load, 2 element, all lanes, LD2R              6              3/2                 L                   -
Q-form
ASIMD load, 3 element, multiple, LD3                8              2/3                 L, V                -
D-form, B/H/S
ASIMD load, 3 element, multiple, LD3                10             2/3                 L, V                -
Q-form, B/H/S
ASIMD load, 3 element, multiple, LD3                10             2/3                 L, V                -
Q-form, D
ASIMD load, 3 element, one lane, LD3                8              2/3                 L, V                -
B/H
ASIMD load, 3 element, one lane, LD3                8              2/3                 L, V                -
S
ASIMD load, 3 element, one lane, LD3                8              2/3                 L, V                -
D
ASIMD load, 3 element, all lanes, LD3R              6              1                   L                   -
D-form, B/H/S
ASIMD load, 3 element, all lanes, LD3R              6              1                   L                   -
D-form, D
ASIMD load, 3 element, all lanes, LD3R              6              1                   L                   -
Q-form, B/H/S
ASIMD load, 3 element, all lanes, LD3R              6              1                   L                   -
Q-form, D
ASIMD load, 4 element, multiple, LD4                8              1/2                 L, V                -
D-form, B/H/S
ASIMD load, 4 element, multiple, LD4                8              1/2                 L, V                -
Q-form, B/H/S
ASIMD load, 4 element, multiple, LD4                8              1/2                 L, V                -
Q-form, D
ASIMD load, 4 element, one lane, LD4                8              1/2                 L, V                -
B/H
ASIMD load, 4 element, one lane, LD4                8              1/2                 L, V                -
S
ASIMD load, 4 element, one lane, LD4                8              1/2                 L, V                -
D
ASIMD load, 4 element, all lanes, LD4R              8              2/3                 L, V                -
D-form, B/H/S
ASIMD load, 4 element, all lanes, LD4R              8              1/2                 L, V                -
D-form, D
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
ASIMD load, 4 element, all lanes, LD4R                 8              2/3                 L, V                -
Q-form, B/H/S
ASIMD load, 4 element, all lanes, LD4R                 8              1/2                 L, V                -
Q-form, D
(ASIMD load, writeback form)       -                   -              -                   I                   1
```

Notes:
1. Writeback forms of load instructions require an extra µOP to update the base address. This update is typically performed
in parallel with the load µOP.

### 3.21 ASIMD store instructions

Stores MOPs are split into store address and store data µOPs. Once executed, stores are buffered
and committed in the background.

Table 3-20 AArch64 ASIMD store instructions

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
ASIMD store, 1 element,            ST1                 2              2                   L01, V              -
multiple, 1 reg, D-form
ASIMD store, 1 element,            ST1                 2              2                   L01, V              -
multiple, 1 reg, Q-form
ASIMD store, 1 element,            ST1                 2              2                   L01, V              -
multiple, 2 reg, D-form
ASIMD store, 1 element,            ST1                 2              2                   L01, V              -
multiple, 2 reg, Q-form
ASIMD store, 1 element,            ST1                 2              1                   L01, V              -
multiple, 3 reg, D-form
ASIMD store, 1 element,            ST1                 2              1                   L01, V              -
multiple, 3 reg, Q-form
ASIMD store, 1 element,            ST1                 2              1                   L01, V              -
multiple, 4 reg, D-form
ASIMD store, 1 element,            ST1                 2              1                   L01, V              -
multiple, 4 reg, Q-form
ASIMD store, 1 element, one        ST1                 2              2                   L01, V              -
lane, B/H/S
ASIMD store, 1 element, one        ST1                 2              2                   L01, V              -
lane, D
ASIMD store, 2 element,            ST2                 2              2                   V, L01              -
multiple, D-form, B/H/S
ASIMD store, 2 element,            ST2                 2              2                   V, L01              -
multiple, Q-form, B/H/S
Instruction Group                  AArch64              Exec          Execution           Utilized            Notes
                                   Instructions         Latency       Throughput          Pipelines
ASIMD store, 2 element,            ST2                  2             2                   V, L01              -
multiple, Q-form, D
ASIMD store, 2 element, one        ST2                  2             2                   V, L01              -
lane, B/H/S
ASIMD store, 2 element, one        ST2                  2             2                   V, L01              -
lane, D
ASIMD store, 3 element,            ST3                  4             1                   V, L01              -
multiple, D-form, B/H/S
ASIMD store, 3 element,            ST3                  4             2/3                 V, L01              -
multiple, Q-form, B/H/S
ASIMD store, 3 element,            ST3                  2             2/3                 V, L01              -
multiple, Q-form, D
ASIMD store, 3 element, one        ST3                  2             1                   V, L01              -
lane, B/H
ASIMD store, 3 element, one        ST3                  2             1                   V, L01              -
lane, S
ASIMD store, 3 element, one        ST3                  2             1                   V, L01              -
lane, D
ASIMD store, 4 element,            ST4                  4             1                   V, L01              -
multiple, D-form, B/H/S
ASIMD store, 4 element,            ST4                  4             1/2                 V, L01              -
multiple, Q-form, B/H/S
ASIMD store, 4 element,            ST4                  2             1                   V, L01              -
multiple, Q-form, D
ASIMD store, 4 element, one        ST4                  2             1                   V, L01              -
lane, B/H/S
ASIMD store, 4 element, one        ST4                  2             1                   V, L01              -
lane, D
(ASIMD store, writeback form)      -                    -             -                   I                   1
```

Notes:
1. Writeback forms of store instructions require an extra µOP to update the base address. This update is typically
performed in parallel with the store µOP (update latency shown in parentheses).

### 3.22 Cryptography extensions

Table 3-21 AArch64 Cryptography extensions

```text
Instruction Group                  AArch64              Exec          Execution           Utilized            Notes
                                   Instructions         Latency       Throughput          Pipelines
Crypto AES ops                     AESD, AESE,          2             2                   V                   -
                                   AESIMC, AESMC
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
Crypto polynomial (64x64)          PMULL (2)           2              1                   V0                  -
multiply long
Crypto SHA1 hash acceleration      SHA1H               2              1                   V0                  -
op
Crypto SHA1 hash acceleration      SHA1C, SHA1M,       4              1                   V0                  -
ops                                SHA1P
Crypto SHA1 schedule               SHA1SU0,            2              1                   V0                  -
acceleration ops                   SHA1SU1
Crypto SHA256 hash                 SHA256H,            4              1                   V0                  -
acceleration ops                   SHA256H2
Crypto SHA256 schedule             SHA256SU0,          2              1                   V0                  -
acceleration ops                   SHA256SU1
Crypto SHA512 hash                 SHA512H,            2              1                   V0                  -
acceleration ops                   SHA512H2,
                                   SHA512SU0,
                                   SHA512SU1
Crypto SHA3 ops                    BCAX, EOR3,         2              2                   V                   2
                                   RAX1, XAR
Crypto SM3 ops                     SM3PARTW1,          2              1                   V0                  -
                                   SM3PARTW2SM
                                   3SS1, SM3TT1A,
                                   SM3TT1B,
                                   SM3TT2A,
                                   SM3TT2B
Crypto SM4 ops                     SM4E, SM4EKEY       4              1                   V0                  -
Notes:
1. Adjacent AESE/AESMC instruction pairs and adjacent AESD/AESIMC instruction pairs will exhibit the performance
characteristics described in Section 4.6.
2. SHA3 ops are executed from the ALU pipeline
```

### 3.23 CRC

Table 3-22 AArch64 CRC

```text
Instruction Group                  AArch64             Exec           Execution           Utilized            Notes
                                   Instructions        Latency        Throughput          Pipelines
CRC checksum ops                   CRC32, CRC32C       2              1                   M0                  1
Notes:
1. CRC execution supports late forwarding of the result from a producer µOP to a consumer µOP. This results in a 1 cycle
reduction in latency as seen by the consumer.
```

### 3.24 SVE Predicate instructions

Table 3-23 SVE Predicate Instructions

```text
Instruction Group                 SVE Instruction     Exec           Execution           Utilized            Notes
                                                      Latency        Throughput          Pipelines
Loop control, based on predicate BRKA, BRKB           2              2                   M                   1
Loop control, based on predicate BRKAS, BRKBS         2              2                   M                   1
and flag setting
Loop control, propagating         BRKN, BRKPA,        2              2                   M                   1
                                  BRKPB
Loop control, propagating and     BRKNS, BRKPAS,      2              2                   M                   1
flag setting                      BRKPBS
Loop control, based on GPR        WHILEGE,            2              2                   M                   -
                                  WHILEGT,
                                  WHILEHI,
                                  WHILEHS,
                                  WHILELE,
                                  WHILELO,
                                  WHILELS,
                                  WHILELT,
                                  WHILERW,
                                  WHILEWR
Loop terminate                    CTERMEQ,            1              2                   M                   -
                                  CTERMNE
Predicate counting scalar         ADDPL, ADDVL,       1              4                   I                   -
                                  CNTB, CNTH,
                                  CNTW, CNTD,
                                  DECB, DECH,
                                  DECW, DECD,
                                  INCB, INCH,
                                  INCW, INCD,
                                  RDVL, SQDECB,
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
Predicate counting scalar,        INC, DEC            1              4                   I
ALL, {1,2,4}
Instruction Group                     SVE Instruction     Exec           Execution           Utilized            Notes
                                                          Latency        Throughput          Pipelines
Predicate counting scalar, active CNTP, DECP,             2              2                   M                   -
predicate                         INCP, SQDECP,
                                  SQINCP,
                                  UQDECP,
                                  UQINCP
Predicate counting vector, active DECP, INCP,             7              1                   M, M0, V            -
predicate                         SQDECP,
                                  SQINCP,
                                  UQDECP,
                                  UQINCP
Predicate logical                     AND, BIC, EOR,      1              2                   M
                                      MOV, NAND,
                                      NOR, NOT, ORN,
                                      ORR
Predicate logical, flag setting       ANDS, BICS,         1              2                   M
                                      EORS, MOV,
                                      NANDS, NORS,
                                      NOTS, ORNS,
                                      ORRS
Predicate reverse                     REV                 2              2                   M                   -
Predicate select                      SEL                 1              2                   M                   -
Predicate set                         PFALSE, PTRUE       2              2                   M                   1
Predicate set/initialize, set flags   PTRUES              2              2                   M                   1
Predicate find first/next             PFIRST, PNEXT       2              2                   M                   -
Predicate test                        PTEST               1              2                   M                   -
Predicate transpose                   TRN1, TRN2          2              2                   M                   -
Predicate unpack and widen            PUNPKHI,            2              2                   M                   -
                                      PUNPKLO
Predicate zip/unzip                   ZIP1, ZIP2, UZP1, 2                2                   M                   -
                                      UZP2
Notes:
1. Operation leading to all, none element active are optimized in rename stage pipeline, execution latency and throughput
are then not representative.
```

### 3.25 SVE integer instructions

Table 3-24 SVE integer instructions

```text
Instruction Group                     SVE Instruction     Exec           Execution           Utilized            Notes
                                                          Latency        Throughput          Pipelines
Arithmetic, absolute diff             SABD, UABD          2              2                   V                   -
Arithmetic, absolute diff accum       SABA, UABA          4(1)           1                   V1                  1
Arithmetic, absolute diff accum       SABALB, SABALT, 4(1)               1                   V1                  1
long                                  UABALB,
                                      UABALT
Instruction Group                  SVE Instruction     Exec           Execution           Utilized            Notes
                                                       Latency        Throughput          Pipelines
Arithmetic, absolute diff long     SABDLB,             2              2                   V                   -
                                   SABDLT,
                                   UABDLB,
                                   UABDLT
Arithmetic, basic                  ABS, ADD, ADR, 2                   2                   V                   -
                                   CNOT, NEG,
                                   SADDLB,
                                   SADDLBT,
                                   SADDLT,
                                   SADDWB,
                                   SADDWT,
                                   SHADD, SHSUB,
                                   SHSUBR,
                                   SSUBLB,
                                   SSUBLBT,
                                   SSUBLT,
                                   SSUBLTB,
                                   SSUBWB,
                                   SSUBWT, SUB,
                                   SUBHNB,
                                   SUBHNT, SUBR,
                                   UADDLB,
                                   UADDLT,
                                   UADDWB,
                                   UADDWT,
                                   UHADD, UHSUB,
                                   UHSUBR,
                                   USUBLB,
                                   USUBLT,
                                   USUBWB,
                                   USUBWT
Arithmetic, complex                ADDHNB,       2                    2                   V                   -
                                   ADDHNT,
                                   RADDHNB,
                                   RADDHNT,
                                   RSUBHNB,
                                   RSUBHNT,
                                   SQABS, SQADD,
                                   SQNEG, SQSUB,
                                   SQSUBR,
                                   SRHADD,
                                   SUQADD,
                                   UQADD, UQSUB,
                                   UQSUBR,
                                   USQADD,
                                   URHADD
Arithmetic, large integer          ADCLB, ADCLT,       2              2                   V                   -
                                   SBCLB, SBCLT
Arithmetic, pairwise add           ADDP                2              2                   V                   -
Arithmetic, pairwise add and       SADALP,             4(1)           1                   V1                  1
accum long                         UADALP
Arithmetic, shift                  ASR, ASRR, LSL,     2              1                   V1                  -
                                   LSLR, LSR, LSRR
Instruction Group                    SVE Instruction    Exec           Execution           Utilized            Notes
                                                        Latency        Throughput          Pipelines
Arithmetic, shift and accumulate SRSRA, SSRA,           4(1)           1                   V1                  1
                                 URSRA, USRA
Arithmetic, shift by immediate       SHRNB, SHRNT, 2                   1                   V1                  -
                                     SSHLLB, SSHLLT,
                                     USHLLB, USHLLT
Arithmetic, shift by immediate       SLI, SRI           2              1                   V1                  -
and insert
Arithmetic, shift complex            RSHRNB,        4                  1                   V1                  -
                                     RSHRNT,
                                     SQRSHL,
                                     SQRSHLR,
                                     SQRSHRNB,
                                     SQRSHRNT,
                                     SQRSHRUNB,
                                     SQRSHRUNT,
                                     SQSHL, SQSHLR,
                                     SQSHLU,
                                     SQSHRNB,
                                     SQSHRNT,
                                     SQSHRUNB,
                                     SQSHRUNT,
                                     UQRSHL,
                                     UQRSHLR,
                                     UQRSHRNB,
                                     UQRSHRNT,
                                     UQSHL, UQSHLR,
                                     UQSHRNB,
                                     UQSHRNT
Arithmetic, shift right for divide   ASRD               4              1                   V1                  -
Arithmetic, shift rounding           SRSHL, SRSHLR,     4              1                   V1                  -
                                     SRSHR, URSHL,
                                     URSHLR, URSHR
Bit manipulation                     BDEP, BEXT,        4              1/2                 V0                  -
                                     BGRP
Bitwise select                       BSL, BSL1N,        2              2                   V                   -
                                     BSL2N, NBSL
Count/reverse bits                   CLS, CLZ, CNT,     2              2                   V                   -
                                     RBIT
Broadcast logical bitmask            DUPM, MOV          2              2                   V                   -
immediate to vector
Compare and set flags                CMPEQ, CMPGE, 2                   2                   V
                                     CMPGT, CMPHI,
                                     CMPHS, CMPLE,
                                     CMPLO, CMPLS,
                                     CMPLT, CMPNE
Complex add                          CADD, SQCADD       2              2                   V                   -
Complex dot product 8-bit            CDOT               3(1)           2                   V                   1
element
Instruction Group                  SVE Instruction     Exec           Execution           Utilized            Notes
                                                       Latency        Throughput          Pipelines
Complex dot product 16-bit         CDOT                4(1)           1                   V0                  1
element
Complex multiply-add B, H, S       CMLA                4(1)           1                   V0                  1
element size
Complex multiply-add D             CMLA                5(3)           1/2                 V0                  1
element size
Conditional extract operations,    CLASTA, CLASTB 8                   1                   M0, V               -
scalar form
Conditional extract operations,    CLASTA, CLASTB, 2                  2                   V                   -
SIMD&FP scalar and vector          COMPACT,
forms                              SPLICE
Convert to floating point, 64b to SCVTF, UCVTF         3              1                   V0                  -
float or convert to double
Convert to floating point, 32b to SCVTF, UCVTF         4              1/2                 V0                  -
single or half
Convert to floating point, 16b to SCVTF, UCVTF         6              1/4                 V0                  -
half
Copy, scalar                       CPY                 5              1                   M0, V
Copy, scalar SIMD&FP or imm        CPY                 2              2                   V
Divides, 32 bit                    SDIV, SDIVR,        8              1/8                 V0                  2
                                   UDIV, UDIVR
Divides, 64 bit                    SDIV, SDIVR,        16             1/16                V0                  2
                                   UDIV, UDIVR
Dot product, 8 bit                 SDOT, UDOT          3(1)           2                   V                   1
Dot product, 8 bit, using signed   SUDOT, USDOT        3(1)           2                   V                   1
and unsigned integers
Dot product, 16 bit                SDOT, UDOT          4(1)           1                   V0                  1
Duplicate, immediate and           DUP, MOV            2              2                   V                   -
indexed form
Duplicate, scalar form             DUP, MOV            3              1                   M0                  -
Extend, sign or zero               SXTB, SXTH,         2              2                   V                   -
                                   SXTW, UXTB,
                                   UXTH, UXTW
Extract                            EXT                 2              2                   V                   -
Extract narrow saturating          SQXTNB,             4              1                   V1                  -
                                   SQXTNT,
                                   SQXTUNB,
                                   SQXTUNT,
                                   UQXTNB,
                                   UQXTNT
Extract/insert operation, SIMD     LASTA, LASTB,       2              2                   V                   -
and FP scalar form                 INSR
Extract/insert operation, scalar   LASTA, LASTB,       5              2                   V                   -
                                   INSR
Instruction Group                    SVE Instruction     Exec           Execution           Utilized            Notes
                                                         Latency        Throughput          Pipelines
Histogram operations                 HISTCNT,            2              2                   V                   -
                                     HISTSEG
Horizontal operations, B, H, S       INDEX               2              2                   V                   -
form, immediate operands only
Horizontal operations, B, H, S       INDEX               5              1                   M0, V               -
form, scalar, immediate
operands)/ scalar operands only
/ immediate, scalar operands
Horizontal operations, D form,       INDEX               2              2                   V                   -
immediate operands only
Horizontal operations, D form,       INDEX               5              1                   M0, V               -
scalar, immediate operands)/
scalar operands only /
immediate, scalar operands
Logical                              AND, BIC, EON,      2              2                   V                   -
                                     EOR, EORBT,
                                     EORTB, MOV,
                                     NOT, ORN, ORR
Max/min, basic and pairwise          SMAX, SMAXP,        2              2                   V                   -
                                     SMIN, SMINP,
                                     UMAX, UMAXP
                                     UMIN, UMINP
Matching operations                  MATCH,              2              2                   V
                                     NMATCH
Matrix multiply-accumulate           SMMLA, UMMLA, 3(1)                 2                   V                   1
                                     USMMLA
Move prefix                          MOVPRFX             2              2                   V                   -
Multiply, B, H, S element size       MUL, SMULH,         4              1                   V0                  -
                                     UMULH
Multiply, D element size             MUL, SMULH,         5              1/2                 V0                  -
                                     UMULH
Multiply long                        SMULLB,             4              1                   V0                  -
                                     SMULLT,
                                     UMULLB,
                                     UMULLT
Multiply accumulate, B, H, S         MLA, MLS            4(1)           1                   V0                  1
element size
Multiply accumulate, D element       MLA, MLS, MAD,      5(3)           1/2                 V0                  1
size                                 MSB,
Multiply accumulate long             SMLALB,         4(1)               1                   V0                  1
                                     SMLALT,
                                     SMLSLB, SMLSLT,
                                     UMLALB,
                                     UMLALT,
                                     UMLSLB,
                                     UMLSLT
Instruction Group                  SVE Instruction    Exec           Execution           Utilized            Notes
                                                      Latency        Throughput          Pipelines
Multiply accumulate saturating     SQDMLALB,          4(2)           1                   V0                  3
doubling long regular              SQDMLALT,
                                   SQDMLALBT,
                                   SQDMLSLB,
                                   SQDMLSLT,
                                   SQDMLSLBT
Multiply saturating doubling       SQDMULH            4              1                   V0                  -
high, B, H, S element size
Multiply saturating doubling       SQDMULH            5              1/2                 V0                  -
high, D element size
Multiply saturating doubling       SQDMULLB,          4              1                   V0                  -
long                               SQDMULLT
Multiply saturating rounding       SQRDMLAH,          4(2)           1                   V0                  3
doubling regular/complex           SQRDMLSH,
accumulate, B, H, S element size   SQRDCMLAH
Multiply saturating rounding       SQRDMLAH,          5(3)           1/2                 V0                  3
doubling regular/complex           SQRDMLSH,
accumulate, D element size         SQRDCMLAH
Multiply saturating rounding       SQRDMULH           4              1                   V0                  -
doubling regular/complex, B, H,
S element size
Multiply saturating rounding       SQRDMULH           5              1/2                 V0                  -
doubling regular/complex, D
element size
Multiply/multiply long, (8x8)      PMUL, PMULLB,      2              1                   V0                  -
polynomial                         PMULLT
Predicate counting vector          CNT, DECB,         2              2                   V                   -
                                   DECH, DECW,
                                   DECD, INCB,
                                   INCH, INCW,
                                   INCD, SQDECB,
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
Reciprocal estimate for B          URECPE,            4              1                   V0
                                   URSQRTE
Reciprocal estimate for H          URECPE,            6              1/2                 V0
                                   URSQRTE
Instruction Group                   SVE Instruction     Exec           Execution           Utilized            Notes
                                                        Latency        Throughput          Pipelines
Reduction, arithmetic, B form       SADDV, UADDV,       8              1/2                 V, V1               4
                                    SMAXV, SMINV,
                                    UMAXV, UMINV
Reduction, arithmetic, H form       SADDV, UADDV,       7              1                   V, V1               4
                                    SMAXV, SMINV,
                                    UMAXV, UMINV
Reduction, arithmetic, S form       SADDV, UADDV,       4              2                   V
                                    SMAXV, SMINV,
                                    UMAXV, UMINV
Reduction, logical                  ANDV, EORV,         5              1                   V, V1               -
                                    ORV

Reverse, vector                     REV, REVB,          2              2                   V                   -
                                    REVH, REVW
Select, vector form                 MOV, SEL            2              2                   V                   -
Table lookup                        TBL                 2              2                   V                   -
Table lookup extension              TBX                 2              2                   V                   -
Transpose, vector form              TRN1, TRN2          2              2                   V                   -
Unpack and extend                   SUNPKHI,            2              2                   V                   -
                                    SUNPKLO,
                                    UUNPKHI,
                                    UUNPKLO
Zip/unzip                           UZP1, UZP2,         2              2                   V                   -
                                    ZIP1, ZIP2
Notes:
1. SVE accumulate pipelines support late-forwarding of accumulate operands from similar µOPs, allowing a typical sequence
of such µOPs to issue one every N cycles (accumulate latency N shown in parentheses).
2. SVE integer divide operations are now performed using a fully pipelined data path.
3. Same as 2 except that for saturating instructions require an extra cycle of latency for late-forwarding accumulate
operands.
4. Signed Additions need 2 cycles more
```

### 3.26 SVE floating-point instructions

Table 3-25 SVE floating-point instructions

```text
Instruction Group                   SVE Instruction     Exec           Execution           Utilized            Notes
                                                        Latency        Throughput          Pipelines
Floating point absolute             FABD, FABS          2              2                   V                   -
value/difference
Floating point arithmetic           FADD, FNEG,         2              2                   V                   -
                                    FSUB, FSUBR
Floating point associative add,     FADDA               16             1/4                 V                   -
F16
Instruction Group                   SVE Instruction     Exec           Execution           Utilized            Notes
                                                        Latency        Throughput          Pipelines
Floating point associative add,     FADDA               8              1/2                 V                   -
F32
Floating point associative add,     FADDA               4              1                   V                   -
F64
Floating point compare              FACGE, FACGT, 2                    2                   V                   -
                                    FACLE, FACLT,
                                    FCMEQ, FCMGE,
                                    FCMGT, FCMLE,
                                    FCMLT, FCMNE,
                                    FCMUO
Floating point complex add          FCADD               3              2                   V                   -
Floating point complex multiply     FCMLA               4(2)           2                   V                   1
add
Floating point convert, long or     FCVT, FCVTLT,       4              1/2                 V0                  -
narrow (F16 to F32 or F32 to        FCVTNT
F16)
Floating point convert, long or FCVT, FCVTLT,           3              1                   V0                  -
narrow (F16 to F64, F32 to F64, FCVTNT
F64 to F32 or F64 to F16)
Floating point convert, round to    FCVTX,              3              1                   V0                  -
odd                                 FCVTXNT
Floating point base2 log, F16       FLOGB               6              1/4                 V0
Floating point base2 log, F32       FLOGB               4              1/2                 V0
Floating point base2 log, F64       FLOGB               3              1                   V0
Floating point convert to integer, FCVTZS,              6              1/4                 V0                  -
F16                                FCVTZU
Floating point convert to integer, FCVTZS,              4              1/2                 V0                  -
F32                                FCVTZU
Floating point convert to integer, FCVTZS,              3              1                   V0                  -
F64                                FCVTZU
Floating point copy                 FCPY, FDUP,         2              2                   V                   -
                                    FMOV
Floating point divide, F16          FDIV, FDIVR         12             1/8                 V0                  2
Floating point divide, F32          FDIV, FDIVR         10             1/4                 V0                  2
Floating point divide, F64          FDIV, FDIVR         13             1/2                 V0                  2
Floating point arith, min/max       FADDP, FMAXP,       3              2                   V
pairwise                            FMAXNMP,
                                    FMINP,
                                    FMINNMP
Floating point min/max              FMAX, DMIN,         2              2                   V                   -
                                    FMAXNM,
                                    FMINNM
Floating point multiply             FSCALE, FMUL,       3              2                   V                   -
                                    FMULX
Instruction Group                   SVE Instruction     Exec           Execution           Utilized            Notes
                                                        Latency        Throughput          Pipelines
Floating point multiply             FMLA, FMLS,   4(2)                 2                   V                   1
accumulate                          FMAD, FMSB,
                                    FNMAD, FNMLA,
                                    FNMLS, FNMSB
Floating point multiply add/sub     FMLALB,        4(2)                2                   V                   1
accumulate long                     FMLALT,
                                    FMLSLB, FMLSLT
Floating point reciprocal           FRECPE,             6              1/4                 V0                  -
estimate, F16                       FRECPX,
                                    FRSQRTE
Floating point reciprocal           FRECPE,             4              1/2                 V0                  -
estimate, F32                       FRECPX,
                                    FRSQRTE
Floating point reciprocal           FRECPE,             3              1                   V0                  -
estimate, F64                       FRECPX,
                                    FRSQRTE
Floating point reciprocal step      FRECPS,             4              2                   V                   -
                                    FRSQRTS
Floating point reduction, F16       FADDV,              6              2/3                 V                   -
                                    FMAXNMV,
                                    FMAXV,
                                    FMINNMV,
                                    FMINV
Floating point reduction, F32       FADDV,              4              1                   V                   -
                                    FMAXNMV,
                                    FMAXV,
                                    FMINNMV,
                                    FMINV
Floating point reduction, F64       FADDV,              2              2                   V                   -
                                    FMAXNMV,
                                    FMAXV,
                                    FMINNMV,
                                    FMINV
Floating point round to integral,   FRINTA, FRINTM, 6                  1/4                 V0                  -
F16                                 FRINTN, FRINTP,
                                    FRINTX, FRINTZ
Floating point round to integral,   FRINTA, FRINTM, 4                  1/2                 V0                  -
F32                                 FRINTN, FRINTP,
                                    FRINTX, FRINTZ
Floating point round to integral,   FRINTA, FRINTM, 3                  1                   V0                  -
F64                                 FRINTN, FRINTP,
                                    FRINTX, FRINTZ
Floating point square root, F16     FSQRT               12             1/8                 V0                  2
Floating point square root, F32     FSQRT               10             1/4                 V0                  2
Floating point square root F64      FSQRT               13             1/2                 V0                  2
Floating point trigonometric        FEXPA               2              2                   V
exponentiation
Instruction Group                  SVE Instruction     Exec           Execution           Utilized            Notes
                                                       Latency        Throughput          Pipelines
Floating point trigonometric       FTMAD               4              2                   V
multiply add
Floating point trigonometric,      FTSMUL, FTSSEL 3                   2                   V                   -
miscellaneous
Notes:
1. SVE multiply-accumulate pipelines support late-forwarding of accumulate operands from similar µOPs, allowing a typical
sequence of floating-point multiply-accumulate µOPs to issue one every N cycles (accumulate latency N shown in
parentheses).
2. SVE FP divide and square root operations are now performed using a fully pipelined data path.
```

### 3.27 SVE BFloat16 (BF16) instructions

Table 3-26 SVE Bfloat16 (BF16) instructions

```text
Instruction Group                  SVE Instruction     Exec           Execution           Utilized            Notes
                                                       Latency        Throughput          Pipelines
Convert, F32 to BF16               BFCVT,              4              1/2                 V0                  -
                                   BFCVTNT
Dot product                        BFDOT               4(2)           2                   V                   1
Matrix multiply accumulate         BFMMLA              5(3)           2                   V                   1
Multiply accumulate long           BFMLALB,            4(2)           2                   V                   1
                                   BFMLALT
Notes:
1. SVE pipelines that execute these instructions support late-forwarding of accumulate operands from similar µOPs,
allowing a typical sequence of µOPs to issue one every N cycles (accumulate latency N shown in parentheses).
```

### 3.28 SVE Load instructions

The latencies shown assume the memory access hits in the Level 1 Data Cache and represent the
maximum latency to load all the vector registers written by the instruction.

Table 3-27 SVE Load instructions

```text
Instruction Group                  SVE Instruction     Exec           Execution           Utilized            Notes
                                                       Latency        Throughput          Pipelines
Load vector                        LDR                 6              3                   L                   -
Load predicate                     LDR                 7              2                   L, M                -
Contiguous load, scalar + imm      LD1B, LD1D,         6              3                   L                   -
                                   LD1H, LD1W,
                                   LD1SB, LD1SH,
                                   LD1SW,
Instruction Group                  SVE Instruction     Exec           Execution           Utilized            Notes
                                                       Latency        Throughput          Pipelines
Contiguous load, scalar + scalar   LD1B, LD1D,         6              3                   L                   -
                                   LD1H, LD1W,
                                   LD1SB, LD1SH
                                   LD1SW
Contiguous load broadcast,         LD1RB, LD1RH,       6              3                   L                   -
scalar + imm                       LD1RD, LD1RW,
                                   LD1RSB,
                                   LD1RSH,
                                   LD1RSW,
                                   LD1RQB,
                                   LD1RQD,
                                   LD1RQH,
                                   LD1RQW
Contiguous load broadcast,         LD1RQB,             6              3                   L                   -
scalar + scalar                    LD1RQD,
                                   LD1RQH,
                                   LD1RQW
Non temporal load, scalar + imm LDNT1B,                6              3                   L                   -
                                LDNT1D,
                                LDNT1H,
                                LDNT1W
Non temporal load, scalar +        LDNT1B,             6              3                   L                   -
scalar                             LDNT1D,
                                   LDNT1H,
                                   LDNT1W
Non temporal gather load,          LDNT1B,             7              3/4                 L                   -
vector + scalar 32-bit element     LDNT1H,
size                               LDNT1W,
                                   LDNT1SB,
                                   LDNT1SH
Non temporal gather load,          LDNT1B,             6              4/5                 L                   -
vector + scalar 64-bit element     LDNT1D,
size                               LDNT1H,
                                   LDNT1W,
                                   LDNT1SB,
                                   LDNT1SH,
                                   LDNT1SW
Contiguous first faulting load,    LDFF1B,             6              3                   L                   -
scalar + scalar                    LDFF1D,
                                   LDFF1H,
                                   LDFF1W,
                                   LDFF1SB,
                                   LDFF1SD,
                                   LDFF1SH
                                   LDFF1SW
Instruction Group                 SVE Instruction     Exec           Execution           Utilized            Notes
                                                      Latency        Throughput          Pipelines
Contiguous non faulting load,     LDNF1B,             6              3                   L                   -
scalar + imm                      LDNF1D,
                                  LDNF1H,
                                  LDNF1W,
                                  LDNF1SB,
                                  LDNF1SH,
                                  LDNF1SW
Contiguous Load two structures LD2B, LD2D,            8              2                   V, L                -
to two vectors, scalar + imm   LD2H, LD2W
Contiguous Load two structures LD2B, LD2D,            8              2                   V, L
to two vectors, scalar + scalar LD2H, LD2W
Contiguous Load three             LD3D                8              2/3                 V, L                -
structures to three vectors,
scalar + imm
Contiguous Load three             LD3B, LD3H,         10             1/3                 V, L
structures to three vectors,      LD3W
scalar + imm
Contiguous Load three             LD3D                9              2/3                 V, L, I             -
structures to three vectors,
scalar + scalar
Contiguous Load three             LD3B, LD3W,         11             1/3                 V, L, I             -
structures to three vectors,      LD3H
scalar + scalar
Contiguous Load four structures LD4D                  8              1/2                 V, L                -
to four vectors, scalar + imm
Contiguous Load four structures LD4B, LD4H,           12             2/5                 V, L                -
to four vectors, scalar + imm   LD4W
Contiguous Load four structures LD4D                  9              1/2                 L, V, I             -
to four vectors, scalar + scalar
Contiguous Load four structures LD4B, LD4H,           13             2/5                 L, V, I             -
to four vectors, scalar + scalar LD4W
Gather load, vector + imm, 32-    LD1B, LD1H,         7              3/4                 L                   -
bit element size                  LD1W, LD1SB,
                                  LD1SH, LD1SW,
                                  LDFF1B,
                                  LDFF1H,
                                  LDFF1W,
                                  LDFF1SB,
                                  LDFF1SH,
                                  LDFF1SW
Instruction Group                  SVE Instruction     Exec           Execution           Utilized            Notes
                                                       Latency        Throughput          Pipelines
Gather load, vector + imm, 64-     LD1B, LD1D,    6                   4/5                 L                   -
bit element size                   LD1H, LD1W,
                                   LD1SB, LD1SH,
                                   LD1SW, LDFF1B,
                                   LDFF1D
                                   LDFF1H,
                                   LDFF1W,
                                   LDFF1SB,
                                   LDFF1SD,
                                   LDFF1SH,
                                   LDFF1SW
Gather load, 32-bit scaled,        LD1H, LD1SH,   7                   3/4                 L                   -
unscaled offset                    LDFF1H,
                                   LDFF1SH, LD1W,
                                   LDFF1W,
                                   LDFF1SW
Gather load, 32-bit unpacked       LD1B, LD1SB,   6                   4/5                 L                   -
unscaled offset, 64 bit scaled,    LDFF1B,
unscaled offset                    LDFF1SB, LD1D,
                                   LDFF1D, LD1H,
                                   LD1SH, LDFF1H,
                                   LDFF1SH, LD1W,
                                   LD1SW,
                                   LDFF1W,
                                   LDFF1SW
Gather load, 32-bit unscaled       LD1B, LD1SB,        7              3/4                 L
offset                             LDFF1B,
                                   LDFF1SB
Gather load, 32-bit unpacked       LD1B, LD1SB,        6              4/5                 L                   -
unscaled offset, 64 bit unscaled   LDFF1B,
offset                             LDFF1SB
```

### 3.29 SVE Store instructions

Table 3-28 SVE Store instructions

```text
Instruction Group                  SVE Instruction     Exec           Execution           Utilized            Notes
                                                       Latency        Throughput          Pipelines
Store from predicate reg           STR                 1              2                   L01                 -
Store from vector reg              STR                 2              2                   L01, V              -
Contiguous store, scalar + imm     ST1B, ST1H,         2              2                   L01, V              -
                                   ST1D, ST1W
Contiguous store, scalar + scalar ST1H                 2              2                   L01, I, V           -
Contiguous store, scalar + scalar ST1B, ST1D,          2              2                   L01, V              -
                                  ST1W
Instruction Group                 SVE Instruction     Exec           Execution           Utilized            Notes
                                                      Latency        Throughput          Pipelines
Contiguous store two structures ST2B, ST2H,           2              2                   L01, V              -
from two vectors, scalar + imm  ST2D, ST2W
Contiguous store two structures ST2B, ST2D,           2              2                   L01, V              -
from two vectors, scalar + scalar ST2H, ST2W
Contiguous store three            ST3B, ST3D,         4              2/3                 L01, V              -
structures from three vectors,    ST3H, ST3W
scalar + imm
Contiguous store three            ST3D                3              2/3                 L01, V
structures from three vectors,
scalar + imm
Contiguous store three            ST3B, ST3H,         4              2/3                 L01, I, V           -
structures from three vectors,    ST3W
scalar + scalar
Contiguous store three            ST3D                3              2/3                 L01, I, V
structures from three vectors,
scalar + scalar
Contiguous store four             ST4B, ST4H,         6              2/3                 L01, V              -
structures from four vectors,     ST4W
scalar + imm
Contiguous store four             ST4D                3              1/2                 L01, V
structures from four vectors,
scalar + imm
Contiguous store four             ST4D                3              1/2                 L01, I, V
structures from four vectors,
scalar + scalar
Contiguous store four             ST4B, ST4H,         6              2/3                 L01, I, V           -
structures from four vectors,     ST4W
scalar + scalar
Non temporal store, scalar +      STNT1B,             2              2                   L01, V              -
imm                               STNT1D,
                                  STNT1H,
                                  STNT1W
Non temporal store, scalar +      STNT1B,             2              2                   L01, V              -
scalar                            STNT1D,
                                  STNT1H,
                                  STNT1W
Scatter non temporal store,       STNT1B,             2              1                   L01, V              -
vector + scalar 32-bit element    STNT1H,
size                              STNT1W
Scatter non temporal store,       STNT1B,             2              2                   L01, V              -
vector + scalar 64-bit element    STNT1D,
size                              STNT1H,
                                  STNT1W
Scatter store vector + imm 32-    ST1B, ST1H,         2              1                   L01, V              -
bit element size                  ST1W
Scatter store vector + imm 64-    ST1B, ST1D,         2              2                   L01, V              -
bit element size                  ST1H, ST1W
Instruction Group                      SVE Instruction     Exec           Execution           Utilized            Notes
                                                           Latency        Throughput          Pipelines
Scatter store, 32-bit scaled           ST1H, ST1W          2              1                   L01, V              -
offset
Scatter store, 32-bit unpacked         ST1B, ST1D,         2              2                   L01, V              -
unscaled offset                        ST1H, ST1W
Scatter store, 32-bit unpacked         ST1D, ST1H,         2              2                   L01, V              -
scaled offset                          ST1W
Scatter store, 32-bit unscaled         ST1B, ST1H,         2              1                   L01, V              -
offset                                 ST1W
Scatter store, 64-bit scaled           ST1D, ST1H,         2              2                   L01, V              -
offset                                 ST1W
Scatter store, 64-bit unscaled         ST1B, ST1D,         2              2                   L01, V              -
offset                                 ST1H, ST1W
```

### 3.30 SVE Miscellaneous instructions

Table 3-29 SVE miscellaneous instructions

```text
Instruction Group                      SVE Instruction     Exec           Execution           Utilized            Notes
                                                           Latency        Throughput          Pipelines
Read first fault register,             RDFFR               2              2                   M                   -
unpredicated
Read first fault register,             RDFFR               2              2                   M
predicated
Read first fault register and set      RDFFRS              2              2                   M
flags
Set first fault register               SETFFR              -              -                   -                   1
Write to first fault register          WRFFR               2              1                   M0                  -
Notes:
1. Operation are optimized in rename stage pipeline, execution latency and throughput are then not representative.
```

### 3.31 SVE Cryptographic instructions

Table 3-48 SVE cryptographic instructions

```text
  Instruction Group                     AArch64                Exec           Execution            Utilized              Notes
                                        Instructions           Latency        Throughput           Pipelines
  Crypto AES ops                        AESD, AESE,            2              2                    V                     -
                                        AESIMC, AESMC
  Crypto SHA3 ops                       BCAX, EOR3,            2              2                    V                     -
                                        RAX1, XAR
  Crypto SM4 ops                        SM4E, SM4EKEY          4              1                    V0                    -
```

## 4 Special considerations

### 4.1 Dispatch constraints

Dispatch of µOPs from the in-order portion to the out-of-order portion of the microarchitecture includes several
constraints. It is important to consider these constraints during code generation to maximize the effective dispatch
bandwidth and subsequent execution bandwidth of Cortex-A720 core.

The dispatch stage can process up to 5 MOPs per cycle and dispatch up to 10 µOPs per cycle, with the following limitations
on the number of µOPs of each type that may be simultaneously dispatched.

Up to 4 µOPs utilizing the S or B pipelines
Up to 4 µOPs utilizing the M pipelines
Up to 2 µOPs utilizing the M0 pipelines
Up to 2 µOPs utilizing the V0 pipeline
Up to 2 µOPs utilizing the V1 pipeline
Up to 5 µOPs utilizing the L pipelines

In the event there are more µOPs available to be dispatched in a given cycle than can be supported by the constraints above,
µOPs will be dispatched in oldest to youngest age-order to the extent allowed by the above.

### 4.2 Optimizing general-purpose register spills and fills

Register transfers between general-purpose registers (GPR) and ASIMD registers (VPR) are lower
latency than reads and writes to the cache hierarchy, thus it is recommended that GPR registers be
filled/spilled to the VPR rather to memory, when possible.

### 4.3 Optimizing memory routines

To achieve maximum throughput for memory copy (or similar loops), one should do the following.

Unroll the loop to include multiple load and store operations per iteration, minimizing the overheads of looping.
Align stores on 32B boundary wherever possible.
Use non-writeback forms of LDP and STP instructions interleaving them like shown in the example below:

```asm
Loop_start:
SUBS      x2,x2,#96
LDP       q3,q4,[x1,#0]
STP       q3,q4,[x0,#0]
LDP       q3,q4,[x1,#32]
STP       q3,q4,[x0,#32]
LDP       q3,q4,[x1,#64]
STP       q3,q4,[x0,#64]
ADD       x1,x1,#96
ADD       x0,x0,#96
BGT       Loop_start
```

If the memory locations being copied are non-cacheable, the non-temporal version of LDPQ (LDNPQ) should be used. STPQ
should still be used for the stores.

Similarly, it Is recommended to use LDPQ to achieve maximum throughput for memcmp (memory compare) loops that
compare cacheable memory. LDNPQ should be used for non-cacheable memory.

To achieve maximum throughput on memset, it is recommended that one do the following.

Unroll the loop to include multiple store operations per iteration, minimizing the overheads of looping.

```asm
Loop_start:
STP          q1,q3,[x0,#0]
STP          q1,q3,[x0,#0x20]
STP          q1,q3,[x0,#0x40]
STP          q1,q3,[x0,#0x60]
ADD          x0,x0,#0x80
SUBS         x2,x2,#0x80
B.GT         Loop_start
```

To achieve maximum performance on memset to zero, it is recommended that one use DC ZVA instead of STP. An optimal
routine might look something like the following.

```asm
Loop_start:
SUBS         x2,x2,#0x80
DC           ZVA,x0
ADD          x0,x0,#0x40
DC           ZVA,x0
ADD          x0,x0,#0x40
B.GT         Loop_start
```

### 4.4 Load/Store alignment

The Armv8-A architecture allows many types of load and store accesses to be arbitrarily aligned. The Cortex-A720 core
handles most unaligned accesses without performance penalties. However, there are cases which could reduce bandwidth
or incur additional latency, as described below.

- Load operations that cross a cache-line (64-byte) boundary.
- Quad-word load operations that are not 4B aligned.
- Store operations that cross a 32B boundary.

### 4.5 Store to Load Forwarding

The Cortex-A720 core allows data to be forwarded from store instructions to a load instruction with the restrictions
mentioned below:

Load start address should align with the start or middle address of the older store

Loads of size greater than 8 bytes can get the data forwarded from a maximum of 2 stores. If there are 2 stores, then each
store should forward to either first or second half of the load

Loads of size less than or equal to 4 bytes can get their data forwarded from only 1 store

### 4.6 AES encryption/decryption

Cortex-A720 core can issue two AESE/AESMC/AESD/AESIMC instruction every cycle (fully
pipelined) with an execution latency of two cycles. Plus note, pairs of dependent AESE/AESMC and
AESD/AESIMC instructions are higher performance when they are adjacent in the program code and
both instructions use the same destination register since they are fused (see Section 4.11 on
Instruction Fusion). This means encryption or decryption for at least four data chunks should be
interleaved for maximum performance, reaching then virtually 4 instructions issue rate in this case:

```asm
AESE     data0, key_reg
AESMC data0, data0
AESE     data1, key_reg
AESMC data1, data1
AESE     data2, key_reg
AESMC data2, data2
AESE     data3, key_reg
AESMC data3, data3
AESE     data0, key_reg
AESMC data0, data0
```

...

### 4.7 Region based fast forwarding

The forwarding logic in the V pipelines is optimized to provide optimal latency for instructions which
are expected to commonly forward to one another.

This defined in the following table.

Table 4-1 Optimized INT forwarding regions

```text
Region         Instruction Types                                                                     Notes
1              ASIMD/SVE integer ALU, ASIMD/SVE integer shift, ASIMD/scalar insert and move,         1
               ASIMD/SVE integer abs/cmp/max/min, ASIMD/SVE AES, ASIMD/SVE polynomial
               multiply, ASIMD/SVE integer reduction, SHA3 and PERM instructions in part 3.19
               see Note 2
2              ASIMD/SVE integer mul/mac                                                             2
3              ASIMD/SVE Crypto, SHA1/SHA256                                                         1

Table 4-2 Optimized FP forwarding regions
Region         Instruction Types                                                                     Notes
1              FP/ASIMD/SVE floating-point multiply, FP/ASIMD/SVE floating point multiply-           1
               accumulate, FP/ASIMD/SVE compare, FP/ASIMD/SVE add/sub and PERM
               instructions in part 3.19 see Note 2
2              ASIMD/SVE BFDOT and BFMMLA instructions
```

Notes:
1.   ASIMD/SVE extract narrow, saturating instructions are excluded from this region and ASIMD/SVE integer
reduction are only consumer forward from this region
2.   ASIM/SVE INT multiply accumulate only fast forward to accumulation source

The following instructions are not part of any region:
- FP/ASIMD/SVE convert and rounding instructions that do not write to general purpose registers
- FP div/sqrt
- SVE sdiv, udiv
- FP convert and rounding instructions that do not write to general purpose registers

In addition to the regions mentioned in the table above, all instructions in regions INT1 and FP1 can
fast forward to FP/ASIMD/SVE stores plus FP/ASIMD vector to integer register transfers, ASIMD
converts that write to general purpose registers and PERM instructions in part 3.19 see Note 2.

More special notes about the forwarding region in Table 4-1 Optimized INT forwarding regions:
- Complex shift by immediate/register and shift accumulate instructions cannot be producers (see
sections 3.16 and 3.25) in region INT1.
- Extract narrow, saturating instructions cannot be producers (see sections 3.19 and 3.25) in
region INT1.
- Absolute difference accumulate and pairwise add and accumulate instructions cannot be
producers (see sections 3.16 and 3.25) in region INT1.
More special notes about the forwarding region in Table 4-2 Optimized FP forwarding regions:
- Element sources (the non-vector operand in "by element" multiplies) used by ASIMD/SVE
floating-point multiply and multiply-accumulate operations cannot be consumers.
- For floating-point producer-consumer pairs, the precision of the instructions should match
(single, double or half) in region FP1.
- Pair-wise floating-point instructions cannot be producers or consumers in region FP1.

It is not advisable to interleave instructions belonging to different regions. Also, certain instructions
can only be producers or consumers in a particular region but not both (see footnote for Table 4-1
Optimized INT forwarding regions and Table 4-2 Optimized FP forwarding regions). For example, the
code below interleaves producers and consumers from regions INT1 and INT2. This will result in an
additional latency of 1 cycle as seen by MUL.
INS v27[1], v20[1]- Region INT1 producer but not a region INT2 consumer
MUL v26, v27, v6 – Region INT2

These fast forwarding regions described in Table 4-1 Optimized INT forwarding regions and Table
4-2 Optimized FP forwarding regions are forming two clusters: cluster FP and cluster INT.
Intercluster communication requires one cycle penalty. For example, the code below
FADD v20.2s, v28.2s, v20.2s – Region FP1

```asm
ADD v27, v20, v20- Region INT1 producer but not a region FP1 consumer
```

### 4.8 Branch instruction alignment

Branch instruction and branch target instruction alignment and density can affect performance.

For best performance, prefer placing taken branches towards the end of an aligned 32-byte
instruction memory region and prefer to have branch target pointing toward the beginning of an
aligned 32-byte instruction.

Cortex-A720 core prediction is optimized to handle aligned 32-byte instruction region containing no
branches. For best performance and power efficiency, avoid diluting branches over aligned
instruction regions.

It is preferable to have an aligned 32-byte instruction region containing two branches, to having two
32-byte regions containing one branch each.

To avoid branch prediction limitation, avoid placing a branch as the last instruction of a 4MB aligned
instruction region of code.

### 4.9 FPCR self-synchronization

Programmers and compiler writers should note that writes to the FPCR register are self-
synchronizing, i.e. its effect on subsequent instructions can be relied upon without an intervening
context synchronizing operation.

### 4.10 Special register access

The Cortex-A720 core performs register renaming for general purpose registers to enable speculative and out-of-order
instruction execution. But most special-purpose registers are not renamed. Instructions that read or write non-renamed
registers are subjected to one or more of the following additional execution constraints.

Non-Speculative Execution – Instructions may only execute non-speculatively.
In-Order Execution – Instructions must execute in-order with respect to other similar instructions or in some
cases all instructions.
Flush Side-Effects – Instructions trigger a flush side-effect after executing for synchronization.

The table below summarizes various special-purpose register read accesses and the associated execution constraints or
side-effects.

Table 4-3 Special-purpose register read accesses

```text
  Register Read                           Non-Speculative             In-            Flush Side-Effect                 Notes
                                                                      Order
CurrentEL                               No                          Yes            No                              -
DAIF                                    No                          Yes            No                              -
DLR_EL0                                 No                          Yes            No                              -
DSPSR_EL0                               No                          Yes            No                              -
ELR_*                                   No                          Yes            No                              -
FPCR                                    No                          Yes            No                              -
FPSR                                    Yes                         Yes            No                              2
NZCV                                    No                          No             No                              1
SP_*                                    No                          No             No                              1
SPSel                                   No                          Yes            No                              -
SPSR_*                                  No                          Yes            No                              -
FFR                                     No                          Yes            No                              -
Notes:
1. The NZCV and SP registers are fully renamed.
2. FPSR/FPSCR reads must wait for all prior instructions that may update the status flags to execute and retire.
The table below summarizes various special-purpose register write accesses and the associated execution constraints or
side-effects.
Table 4-3 Special-purpose register write accesses
  Register Write                            Non-Speculative              In-           Flush Side-Effect                Notes
                                                                         Order
DAIF                                      Yes                          Yes           No                             -
DLR_EL0                                   Yes                          Yes           No                             -
DSPSR_EL0                                 Yes                          Yes           No                             -
ELR_*                                     Yes                          Yes           No                             -
FPCR                                      Yes                          Yes           Maybe                          2
FPSR                                      Yes                          Yes           No                             3
NZCV                                      No                           No            No                             1
SP_*                                      No                           No            No                             1
SPSel                                     Yes                          Yes           Yes                            -
SPSR_*                                    Yes                          Yes           No                             -
SETFFR                                    No                           No            No
WRFFR                                     Yes                          Yes           No
Notes:
1. The NZCV and SP registers are fully renamed.
2. If the FPCR write is predicted to change the control field values, it will introduce a barrier which prevents subsequent
instructions from executing. If the FPCR write is predicted to not change the control field values, it will execute without a
barrier but trigger a flush if the values change. If the FPCR write changes the control field NEP it will trigger a flush.
3. FPSR writes must stall at dispatch if another FPSR write is still pending.
```

### 4.11 Instruction fusion

Cortex-A720 core can accelerate certain instruction pairs in an operation called fusion. Specific instruction pairs that can be
fused are as follows:

AESE + AESMC (see Section 4.6 on AES Encryption/Decryption)
AESD + AESIMC (see Section 4.6 on AES Encryption/Decryption)
CMP/CMN (immediate) + B.cond
CMP/CMN (register Rn != ZR) + B.cond
TST (immediate) + B.cond
TST (register) + B.cond
BICS ZR (register) + B.cond
CMP (immediate) + CSEL
CMP (register) + CSEL
CMP (immediate) + CSET
CMP (register) + CSET

```asm
BTI + Integer DP/BR/BLR/RET/B uncond/CBZ/TBZ
```

SHL + SRI (both scalar or both vector)
FCMP + AXFLAG
MOVPRFX + supported SVE instruction

These instruction pairs must be adjacent to each other in program code. For CMP, CMN, TST fusion is allowed for shifted
and/or extended register forms. For CMP, CMN, TST and BICS, there are restrictions on immediate values for both
instructions of the pair for which fusion is supported. Other particular restrictions apply on instruction fusion.

### 4.12 Zero Latency Instructions

A subset of register-to-register move operations, move immediate operations, predicates operations
are executed with zero latency. These instructions do not utilize the scheduling and execution
resources of the machine. These are as follows:

MOV Xd, #{12{1'b0},imm[3:0]}

MOV Xd, XZR

MOV Wd, #{12{1'b0},imm[3:0]}

MOV Wd, WZR

MOV Hd, WZR

MOV Hd, XZR

MOV Sd, WZR

MOV Dd, XZR

MOVI Dd, #0

MOVI Vd.2D, #0

MOV Wd, Wn

MOV Xd, Xn

FMOV Sd, Sn

FMOV Dd, Dn

MOV Vd, Vn (vector)

MOV Zd.D, Zn.D

PTRUE

PFALSE

SETFFR

The MOV Wd, Wn, MOV Xd, Xn and FMOV Sd, Sn, FMOV Dd, Dn, MOV Vd, Vn (vector), MOV Zd.D,
Zn.D instructions may not be executed with zero latency under certain conditions.

### 4.13 TLB-access latencies

A hit in the L1 instruction TLB provides a single CLK cycle access to the translation and returns the PA to the instruction
cache for comparison. It also checks the access permissions to signal an Instruction Abort.

A hit in the L1 data TLB provides a single CLK cycle access to the translation and returns the PA to the data cache for
comparison. It also checks the access permissions to signal a Data Abort.

A miss in the L1 data TLB followed by a hit in the L2 TLB has a 5-cycle penalty compared to a hit in the L1 data TLB. This
penalty can be increased depending on the arbitration of pending requests

### 4.14 Cache-access latencies

The Cortex-A720 core pipeline is optimized for low latency and high bandwidth. The following table
lists the latencies for the different levels of cache.

Table 4-4 Cortex-A720 core cache access latencies

```text
    Scenario                                                Cycle count
Level-1 Cache Hit                                         4 core cycles
Level-2 Cache Hit                                         9 core cycles
Level-3 Cache Hit                                         19.5 core cycles + 14.5 DSU cycles
Level-1 Cache Hit in another Cortex-A720 core in the 38 core cycles + 22.5 DSU cycles
same cluster
Level-2 Cache Hit in another Cortex-A720 core in the 32 core cycles + 22.5 DSU cycles
same cluster
Level-3 Cache Miss, DMC access                            19.5 core cycles + 15.5 DSU cycles + 2 SYS cycles + system
                                                          latency
```

The information in Table 4-4 Cortex-A720 core cache access latencies is based on the assumptions
that:
- Asynchronous bridges are present between core and DSU with 2-stage synchronizers in each
clock domain. Latencies that include crossing the asynchronous boundary to the DSU use average
latencies through the asynchronous bridge.
- The Level-3 cache data RAM latency configuration is the default 1-cycle in, 2-cycles out.
- DSU frequency is 2GHz, asynchronous to 3GHz CPU frequency. Higher DSU frequency might
require extra flops that increase the latency to L3.
- The cluster contains 1-4 cores. Additional cores might require register slices that increase the
latency to L3.

Latencies are specified as load-to-use. This measurement represents the number of cycles from when
a load instruction is in a given pipeline stage to when a dependent instruction is in the same pipeline
stage.

### 4.15 Cache maintenance operation

While using set way invalidation operations on L1 cache, it is recommended that software be written
to traverse the sets in the inner loop and ways in the outer loop.

### 4.16 Memory Tagging - Tagging Performance

To achieve maximum throughput for tag-only, it is recommended that one do the following.

Unroll the loop to include multiple store operations per iteration, minimizing the overheads of looping. Use STGM (or
DCGVA) instruction as shown in the example below:

```asm
Loop_start:
SUBS     x2,x2,#0x80
STGM     x1,[x0]
ADD      x0,x0,#0x40
STGM     x1,[x0]
ADD      x0,x0,#0x40
B.GT     Loop_start
```

To achieve maximum throughput for tag and zeroing out data, it is recommended that one do the following.

Unroll the loop to include multiple store operations per iteration, minimizing the overheads of looping. Use STZGM (or
DCZGVA) instruction as shown in the example below:

```asm
Loop_start:
SUBS     x2,x2,#0x80
STZGM x1,[x0]
ADD      x0,x0,#0x40
STZGM x1,[x0]
ADD      x0,x0,#0x40
B.GT     Loop_start
```

To achieve maximum throughput for tag-loading, it is recommended that one do the following.

Unroll the loop to include multiple load operations per iteration, minimizing the overheads of looping. Use LDGM instruction
as shown in the example below:

```asm
Loop_start:
SUBS     x2,x2,#0x80
LDGM     x1,[x0]
ADD      x0,x0,#0x40
LDGM     x1,[x0]
ADD      x0,x0,#0x40
B.GT   Loop_start
```

Also, it is recommended to use STZGM (or DCZGVA) to set tag if data is not a concern.

### 4.17 Memory Tagging - Synchronous Mode

In synchronous tag checking mode, each store must complete a tag check before the next store can be
executed Thus, performance of stores in synchronous tag checking mode will be diminished.

It is recommended to use asynchronous mode for better performance.

### 4.18 Complex ASIMD and SVE instructions

The bandwidth of the following ASIMD and SVE instructions is limited by decode constraints and it is
advisable to avoid them when high performing code is desired.

ASIMD

LD4R, post-indexed addressing, element size = 64b.

LD4, single 4-element structure, post indexed addressing mode, element size = 64b.

LD4, multiple 4-element structures, quad form, element size less than 64b.

LD4, multiple 4-element structures, quad form, element size less than 64b, , post indexed addressing
mode.

ST4, multiple 4-element structures, quad form, element size less than 64b.

ST4, multiple 4-element structures, quad form, element size = 64b, post indexed addressing mode.

SVE

LD1H gather (scalar + vector addressing) where vector index register is the same as the destination
register and element size = 32. Addressing mode is 32b scaled or unscaled offset.

LD3[B/H] contiguous (scalar + scalar addressing).

LD4[B/H/W] contiguous (scalar + immediate addressing).

LD4[B/H/W] contiguous (scalar + scalar addressing).

LDFF1H gather (scalar + vector addressing) where vector index register is the same as the
destination register and element size = 32. Addressing mode is 32b scaled or unscaled offset.

ST3[B/H/W/D] contiguous (scalar + scalar addressing).

ST4[B/H/D/W] contiguous (scalar + scalar addressing).

### 4.19 MOVPRFX fusion

Under certain conditions, a mechanism called MOVPRFX fusion can be used to accelerate the execution of an instruction
pair that consists of an SVE MOVPRFX instruction immediately followed in program order by an SVE integer, floating point
or BF16 instruction. The list of SVE instructions and the conditions under which this fusion can be applied is mentioned in
the tables below.

Table 4-5 MOVPRFX unpredicated fusion

```text
Instruction Group                    SVE Instruction                                 Notes
Integer Instructions
Arithmetic, absolute difference      SABD, UABD                                      -
Arithmetic, absolute difference      SABA, SABALB, SABALT, UABA, UABALB,             -
accumulate                           UABALT
Arithmetic, basic                    ABS, ADD, CNOT, NEG, SHADD, SHSUB,              For ADD and SUB, only the
                                     SHSUBR, SUB, SUBR, UHADD, UHSUB,                immediate and vector, predicated
                                     UHSUBR                                          forms are fusible.
Arithmetic, complex                  SQABS, SQADD, SQNEG, SQSUB, SQSUBR,             For SQABS, SQSUB, UQADD and
                                     SRHADD, SUQADD, UQADD, UQSUB,                   UQSUB, only the immediate and
                                     UQSUBR, URHADD, USQADD                          vector, predicated forms are
                                                                                     fusible.
Arithmetic, large integer            ADCLB, ADCLT, SBCLB, SBCLT                      -
Arithmetic, shift                    ASR, ASRR, LSL, LSLR, LSR, LSRR                 For ASR, LSL and LSR, only the
                                                                                     immediate, predicated and vector
                                                                                     forms are fusible.
Arithmetic, shift and accumulate SRSRA, SSRA, URSRA, USRA                            -
Arithmetic, shift complex            SQRSHL, SQRSHLR, SQSHL, SQSHLR,                 -
                                     UQRSHL, UQRSHLR, UQSHL, UQSHLR
Arithmetic, shift rounding           SRSHL, SRSHLR, URSHL, URSHLR                    -
Bitwise select                       BSL, BSL1N, BSL2N, NBSL                         -
Count/reverse bits                   CLS, CLZ, CNT, RBIT                             -
Complex add                          CADD, SQCADD                                    -
Complex dot product                  CDOT                                            Only the vector form is fusible.
Complex multiply-add                 CMLA                                            Only the vector form is fusible.
Conditional extract operations       CLASTA, CLASTB                                  Only the vector forms are fusible.
Convert to floating point            SCVTF, UCVTF                                    -
Copy                                 CPY                                             Only the SIMD&FP scalar and
                                                                                     immediate merging forms are
                                                                                     fusible
Divides                              SDIV, SDIVR, UDIV, UDIVR                        -
Dot product                          SDOT, UDOT, SUDOT, USDOT                        Only the vector form is fusible
Extend, sign or zero                 SXTB, SXTH, SXTW, UXTB, UXTH, UXTW              -
Extract/insert operation             INSR                                            Only the SIMD&FP scalar form is
                                                                                     fusible
Instruction Group                    SVE Instruction                                 Notes
Logical                              AND, BIC, EON, EOR, EORBT, EORTB,               For AND, BIC, EOR and ORR, only
                                     MOV, NOT, ORN, ORR                              the immediate and vector,
                                                                                     predicated forms are fusible
Max/min, basic and pairwise          SMAX, SMIN, UMAX, UMIN                          Only the immediate and vector,
                                                                                     predicated forms are fusible
Matrix multiply-accumulate           SMMLA, UMMLA, USMMLA                            -
Multiply                             MUL, SMULH, UMULH                               For MUL, only the immediate and
                                                                                     vector, predicated forms are
                                                                                     fusible. For the others, only the
                                                                                     predicated form is fusible.
Multiply accumulate                  MLA, MLS, MAD, MSB                              For MLA, MLS only the vector
                                                                                     forms are fusible
Multiply accumulate long             SMLALB, SMLALT, SMLSLB, SMLSLT,                 Only the vector form is fusible
                                     UMLALB, UMLALT, UMLSLB, UMLSLT
Multiply accumulate saturating       SQDMLALB, SQDMLALT, SQDMLALBT,                  For SQDMLALB, SQDMLALT,
doubling long regular                SQDMLSLB, SQDMLSLT, SQDMLSLBT                   SQDMLSLB, SQDMLSLT only the
                                                                                     vector forms are fusible
Multiply saturating rounding         SQRDMLAH, SQRDMLSH, SQRDCMLAH                   Only the vector form is fusible
doubling regular/complex
accumulate
Predicate counting, vector form      DECH, DECW, DECD, INCH, INCW, INCD,             Only the vector form is fusible
                                     SQDECH, SQDECW, SQDECD, SQINCH,
                                     SQINCW, SQINCD, UQDECH, UQDECW,
                                     UQDECD, UQINCH, UQINCW, UQINCD
Reciprocal estimate                  URECPE, URSQRTE                                 -
Reverse, vector                      REVB, REVH, REVW                                -
Floating point Instructions
Floating point absolute              FABD, FABS                                      -
value/difference
Floating point arithmetic            FADD, FNEG, FSUB, FSUBR                         For FADD, FSUB, FSUBR only the
                                                                                     immediate and vector, predicated
                                                                                     forms are fusible.
Floating point complex add           FCADD                                           -
Floating point complex multiply      FCMLA                                           Only the vector form is fusible
add
Floating point convert               FCVT, FCVTX                                     -
Floating point base2 log             FLOGB                                           -
Floating point convert to integer FCVTZS, FCVTZU                                     -
Floating point copy                  FCPY, FMOV                                      Only the predicated form is fusible
Floating point divide                FDIV, FDIVR                                     -
Floating point min/max               FMAX, FMIN, FMAXNM, FMINNM                      -
Floating point multiply              FSCALE, FMUL, FMULX                             For FMUL, only the immediate and
                                                                                     vector, predicated forms are
                                                                                     fusible
Instruction Group                   SVE Instruction                                 Notes
Floating point multiply             FMLA, FMLS, FMAD, FMSB, FNMAD,                  For FMLA, FMLS only the vector
accumulate                          FNMLA, FNMLS, FNMSB                             forms are fusible
Floating point multiply add/sub     FMLALB, FMLALT, FMLSLB, FMLSLT                  Only the vector form is fusible
accumulate long
Floating point reciprocal           FRECPX                                          -
estimate
Floating point round to integral    FRINTA, FRINTI, FRINTM, FRINTN, FRINTP, -
                                    FRINTX, FRINTZ
Floating point square root          FSQRT                                           -
Floating point trigonometric        FTMAD                                           -
multiply add
BF16 Instructions
Dot product                         BFDOT                                           Only the vector form is fusible
Matrix multiply accumulate          BFMMLA                                          -
Multiply accumulate long            BFMLALB, BFMLALT                                Only the vector form is fusible
Scalar convert, F32 to BF16         BFCVT                                           -
Cryptographic Instructions
Crypto SHA3 ops                     BCAX, EOR3, XAR                                 -
Table 4-6 MOVPRFX predicated fusion
Instruction Group                   SVE Instruction                                 Notes
Integer Instructions
Arithmetic, absolute difference     SABD, UABD                                      -
Arithmetic, basic                   ABS, ADD, CNOT, NEG, SHADD, SHSUB,              For ADD and SUB, only the vector,
                                    SHSUBR, SUB, SUBR, UHADD, UHSUB,                predicated form is fusible.
                                    UHSUBR
Arithmetic, complex                 SQABS, SQADD, SQNEG, SQSUB, SQSUBR,             For SQABS, SQSUB, UQADD and
                                    SRHADD, SUQADD, UQADD, UQSUB,                   UQSUB, only the vector,
                                    UQSUBR, URHADD, USQADD                          predicated form is fusible.
Arithmetic, shift                   ASR, ASRR, LSL, LSLR, LSR, LSRR                 For ASR, LSL and LSR, only the
                                                                                    predicated and vector forms are
                                                                                    fusible.
Count/reverse bits                  CLS, CLZ, CNT, RBIT                             -
Divides                             SDIV, SDIVR, UDIV, UDIVR                        -
Extend, sign or zero                SXTB, SXTH, SXTW, UXTB, UXTH, UXTW              -
Logical                             AND, BIC, EOR, NOT, ORR                         For AND, BIC, EOR and ORR, only
                                                                                    the vector, predicated form is
                                                                                    fusible
Max/min, basic and pairwise         SMAX, SMIN, UMAX, UMIN                          Only the vector form is fusible
Multiply                            MUL, SMULH, UMULH                               For MUL, only the vector,
                                                                                    predicated form is fusible. For the
                                                                                    others, only the predicated form is
                                                                                    fusible.
Reverse, vector                     REVB, REVH, REVW                                -
Floating point Instructions
Floating point absolute             FABD, FABS                                      -
value/difference
Floating point arithmetic           FADD, FNEG, FSUB, FSUBR                         For FADD, FSUB, FSUBR only the
                                                                                    immediate and vector, predicated
                                                                                    forms are fusible.
Floating point complex add          FCADD                                           -
Floating point divide               FDIV, FDIVR                                     -
Floating point min/max              FMAX, FMIN, FMAXNM, FMINNM                      -
Floating point multiply             FMUL, FMULX                                     For FMUL, only the vector,
                                                                                    predicated form is fusible
Floating point multiply             FMLA, FMLS, FMAD, FMSB, FNMAD,                  For FMLA, FMLS only the vector
accumulate                          FNMLA, FNMLS, FNMSB                             forms are fusible
Floating point multiply add/sub     FMLALB, FMLALT, FMLSLB, FMLSLT                  Only the vector form is fusible
accumulate long
Floating point square root          FSQRT                                           -
Appendix A                                     Revisions
This appendix describes the technical changes between released issues of this document.

Table A-1: Issue 1.0
Change                                                            Location               Affects
First Confidential draft release for r0p0                         -                      r0p0

Table A-2: Issue 2.0
Change                                                            Location               Affects
First Confidential limited access release for r0p0                -                      r0p0

Table A-3: Issue 3.0
Change                                                            Location               Affects
First Confidential draft release for r0p1                         -                      r0p1

Fixes for Reduction instruction of 1 RED uop                      Section 3.16           r0p1

Fixes of PMULL instructions played in AES module                  Section 3.25

Fix of Extend, sign or zero SVE intruction played in PERMS
module

Table A-4: Issue 4.0
Change                                                            Location               Affects
First Confidential early access release for r0p1                  -                      r0p1

Table A-5: Issue 5.0
Change                                                            Location               Affects
Second Confidential early access release for r0p1                 -                      r0p1

Updated product name                                              Throughout             r0p1
                                                                  document

Table A-6: Issue 5.1 and more
Change                                                            Location               Affects
Remove fixes of PMULL instructions played in AES module           Section 3.16 and       r0p1
since played in PMUL                                              3.22
Change                                                            Location               Affects
Update AES encryption/decryption description                      Section 4.6            r0p1

Update FCMLA latency                                              Section 3.17           r0p1

Table A-7: Issue 6.0
Change                                                            Location               Affects
Second Confidential release for r0p2                              -                      r0p2

Updated revision value                                            -                      r0p2

Table A-8: Issue 6.1 and more
Change                                                            Location               Affects
Fix AUT and LDRA latencies                                        Section 3.6            r0p2

Fix core cycles for Level-3 Cache accesses                        Section 4.14           r0p2

Table A-8: Issue 7.0
Change                                                            Location               Affects
First Non-Confidential release for r0p2 - No technical change     -                      r0p2

Updated document status to Non-Confidential                       -                      r0p2

Changed document number to 109720                                 -                      r0p2
```
