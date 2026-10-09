"""Authored, exact-input compatibility edits to the owner's BSD PhysX revision.

These are build-time edits to local dependencies, not vendored SDK files. The
installed API is 3.4.0; the licensed source is 3.4.2. Never rewrite its version
or license to pretend they are the same release.
"""
import hashlib
import re

PIN = "5e42a5f112351a223c19c17bb331e6c55037b8eb"
REPOSITORY = "https://github.com/NVIDIAGameWorks/PhysX-3.4.git"
BASE_SHA256 = {
    "PhysX_3.4/Include/PxSceneDesc.h": "6fda46bb44978eb6dc0cf318a7929094a3bd78a6c45a6e4732b6338f3accf694",
    "PhysX_3.4/Include/PxScene.h": "a15ed484a7021087ef1fa7c8e9ee5c6fe9befcd11ec33f5f2628212ca36f6e04",
    "PhysX_3.4/Include/cooking/PxCooking.h": "18709a5addaba1d6b3df0ffe9f3919b0680dcb47f84d6c9b410b24afc9ec86ea",
    "PhysX_3.4/Include/geometry/PxConvexMeshGeometry.h": "0b0a30d8b11dc063cecee83fd0ef6206a45b542614270b35e7500ba24e2f0281",
    "PhysX_3.4/Source/PhysXExtensions/src/ExtD6JointSolverPrep.cpp": "1289e750823474fbf6e4cb8b4153ebe3e1493f62adf1516b3661b08917b0d3f1",
    "PhysX_3.4/Source/PhysXExtensions/src/ExtConstraintHelper.h": "ea0fc91cfc51b8fa196a48248dbc23b948f6d24531f434cf730f1e322a9f8f98",
}


def once(text, before, after):
    if text.count(before) != 1:
        raise ValueError("Compatibility anchor is not unique: " + before)
    return text.replace(before, after, 1)


def matching_line(text, pattern, replacement=""):
    text, count = re.subn(pattern, replacement, text, flags=re.MULTILINE)
    if count != 1:
        raise ValueError("Compatibility declaration is not unique: " + pattern)
    return text


def adapt(relative, original):
    """Return adapted bytes; refuse changed upstream inputs before any edits."""
    expected = BASE_SHA256[relative]
    if hashlib.sha256(original).hexdigest() != expected:
        raise ValueError("Unreviewed BSD source: " + relative)
    text = original.decode("utf-8")
    # Preserve the complete owner license/copyright preamble byte-for-byte.
    preamble = text[:text.index("#include" if relative.endswith(".cpp") else "#ifndef")]
    changes = []
    if relative.endswith("PxSceneDesc.h"):
        for field in ("kineKineFilteringMode", "staticKineFilteringMode", "solverOffsetSlop"):
            text = matching_line(text, r"^\t[^\r\n;]*\b" + field + r"\s*;\r?\n")
            text = matching_line(text, r"^\t" + field + r"[^\r\n]*,\r?\n")
            changes.append("remove later descriptor field+initializer " + field)
        # Keep plane/query-related fields that ARE in the installed PDB layout.
    elif relative.endswith("PxScene.h"):
        for method in ("setSceneQueryUpdateMode", "getSceneQueryUpdateMode", "sceneQueriesUpdate", "checkQueries", "fetchQueries"):
            text = matching_line(text, r"^\tvirtual[^\r\n]*\b" + method + r"\([^\r\n]*\r?\n")
            changes.append("remove later virtual " + method)
        text = once(text, "virtual PxBounds3\t\t\tgetVisualizationCullingBox()", "virtual const PxBounds3&\tgetVisualizationCullingBox()")
        changes.append("restore visualization getter reference return ABI")
    elif relative.endswith("PxCooking.h"):
        for method in ("getParams", "platformMismatch", "cookTriangleMesh", "validateTriangleMesh", "cookConvexMesh", "validateConvexMesh", "cookHeightField", "createHeightField"):
            text = matching_line(text, r"^(\tvirtual[^\r\n]*\b" + method + r"\([^\r\n]*) const( = 0;\r?)$", r"\1\2")
            changes.append("restore nonconst " + method)
        for kind in ("Triangle", "Convex"):
            text = once(text, "PxPhysicsInsertionCallback& insertionCallback, Px" + kind + "MeshCookingResult::Enum* condition = NULL) const = 0;", "PxPhysicsInsertionCallback& insertionCallback) const = 0;")
            changes.append("restore two-argument const create" + kind + "Mesh")
    elif relative.endswith("PxConvexMeshGeometry.h"):
        # Installed PDB: scale+4, convexMesh+32, meshFlags+40, padding+41.
        # The later margin field fits in size48 but moves flags to44.
        text = once(text, "convexMesh\t(NULL),\n\t\tmaxMargin\t(3.4e38f)", "convexMesh\t(NULL)")
        text = once(text, "\t" + r"\param[in] margin" + "\tThe maximum margin. Used to limit how much PCM shrinks the geometry by in collision detection.\n", "")
        text = once(text, "PxConvexMeshGeometryFlags flags = PxConvexMeshGeometryFlags(),\n\t\t\t\t\t\t\t\t\tfloat margin = 3.4e38f)", "PxConvexMeshGeometryFlags flags = PxConvexMeshGeometryFlags())")
        text = once(text, "\t\tmaxMargin\t(margin),\n", "")
        text = matching_line(text, r"^\tPxReal[^\r\n]*\bmaxMargin;[^\r\n]*\r?\n")
        text = once(text, "\tif (maxMargin < 0.0f)\n\t\treturn false;\n", "")
        changes.append("remove later convex maxMargin field, constructor argument/initializers and validation")
    elif relative.endswith("ExtConstraintHelper.h"):
        text = once(text, "(mRa + errorVec).cross(axis)", "mRa.cross(axis)")
        changes.append("installed locked-linear angular0 lever arm without positional error")
    elif relative.endswith("ExtD6JointSolverPrep.cpp"):
        text = once(text, "data.swingLimit.isSoft() ? 0.0f : data.tqSwingPad", "data.tqSwingPad")
        # Restrict the old double-cone equation to the solver. The BSD visualizer
        # and its tanHalfFromSin helper stay as published by NVIDIA.
        for sine in ("-aZ.dot(bX)", "aY.dot(bX)"):
            text = once(text, "tanHalfFromSin(" + sine + ")", "mhInstalledTanHalfFromSin(" + sine + ")")
        text = once(text, "\tPxU32 D6JointSolverPrep(", "\t// Installed solver uses (1-s*s), without the later square root.\n\tstatic PxReal mhInstalledTanHalfFromSin(PxReal s)\n\t{\n\t\treturn Ps::tanHalf(s, 1.0f - s*s);\n\t}\n\n\tPxU32 D6JointSolverPrep(")
        changes += ["retain soft-cone tqSwingPad", "installed single-swing-free double-cone equation"]
    if not text.startswith(preamble):
        raise ValueError("Owner license preamble changed")
    # New comments/helper use the same newline convention as the licensed input.
    if b"\r\n" in original:
        text = text.replace("\r\n", "\n").replace("\n", "\r\n")
    return text.encode("utf-8"), changes
