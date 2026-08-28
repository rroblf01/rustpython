"""Generic (shallow and deep) copying operations. RustPython patched version."""
import types
import weakref
from copyreg import dispatch_table

class Error(Exception):
    pass
error = Error

__all__ = ["Error", "copy", "deepcopy", "replace"]

# compat for missing types
_EllipsisType = getattr(types, 'EllipsisType', type(Ellipsis))
_NotImplementedType = getattr(types, 'NotImplementedType', type(NotImplemented))
_BuiltinFunctionType = getattr(types, 'BuiltinFunctionType', type(len))
_FunctionType = getattr(types, 'FunctionType', type(lambda: None))
_CodeType = getattr(types, 'CodeType', type((lambda: None).__code__))

_copy_atomic_types = {types.NoneType, int, float, bool, complex, str, tuple,
          bytes, frozenset, type, range, slice, property,
          _BuiltinFunctionType, _EllipsisType,
          _NotImplementedType, _FunctionType, _CodeType,
          weakref.ref, super}
_copy_builtin_containers = {list, dict, set, bytearray}

# RustPython shim: real runtime types differ
try:
    _real_atomic_extra = set()
    _real_copy_extra = set()
    def _rt(o):
        try: return type(o)
        except: return None
    _real_atomic_extra.add(_rt(None)); _real_copy_extra.add(_rt(None))
    _real_atomic_extra.add(_rt(Ellipsis)); _real_copy_extra.add(_rt(Ellipsis))
    _real_atomic_extra.add(_rt(NotImplemented)); _real_copy_extra.add(_rt(NotImplemented))
    for _tp in (int, float, bool, complex, str, bytes):
        _real_atomic_extra.add(_tp); _real_copy_extra.add(_tp)
    _real_atomic_extra.add(type); _real_copy_extra.add(type)
    try: _real_atomic_extra.add(_rt(range(0,10))); _real_copy_extra.add(_rt(range(0,10)))
    except: pass
    try: _real_atomic_extra.add(_rt(slice(1,2))); _real_copy_extra.add(_rt(slice(1,2)))
    except: pass
    try: _real_atomic_extra.add(_rt(property())); _real_copy_extra.add(_rt(property()))
    except: pass
    try: _real_atomic_extra.add(_rt(super)); _real_copy_extra.add(_rt(super))
    except: pass
    _real_copy_extra.add(tuple); _real_copy_extra.add(frozenset)
    try: _real_atomic_extra.add(_rt((lambda: None).__code__)); _real_copy_extra.add(_rt((lambda: None).__code__))
    except: pass
    try: _real_atomic_extra.add(_rt(lambda: None)); _real_copy_extra.add(_rt(lambda: None))
    except: pass
    try: _real_atomic_extra.add(_rt(len)); _real_copy_extra.add(_rt(len))
    except: pass
    try: _real_atomic_extra.add(weakref.ref); _real_copy_extra.add(weakref.ref)
    except: pass
    try:
        class _W: pass
        _o=_W(); _r=weakref.ref(_o)
        _real_atomic_extra.add(_rt(_r)); _real_copy_extra.add(_rt(_r))
    except: pass
    try: _real_atomic_extra.add(types.MethodType)
    except: pass
    _copy_atomic_types = _copy_atomic_types | _real_copy_extra
    _atomic_types_for_shim = _real_atomic_extra
except Exception:
    _atomic_types_for_shim = set()

def _is_issubclass(c, b):
    try:
        return issubclass(c, b)
    except TypeError:
        return False

def _is_overridden_getattribute(cls):
    try:
        return cls.__getattribute__ is not object.__getattribute__
    except: return False

def _copy_shallow_fallback(x, cls):
    if isinstance(x, list):
        try:
            y = cls.__new__(cls, x)
        except Exception:
            try:
                y = cls.__new__(cls)
                y.extend(x)
            except Exception:
                y = cls.__new__(cls)
        if hasattr(x, '__dict__'):
            try: y.__dict__.update(x.__dict__)
            except: pass
        slots = getattr(cls,'__slots__',None)
        if slots is not None:
            if isinstance(slots,str): slots=(slots,)
            for sl in slots:
                if isinstance(sl,str) and hasattr(x,sl):
                    try: setattr(y,sl,getattr(x,sl))
                    except: pass
        return y
    if isinstance(x, dict):
        if isinstance(x, _weakref.WeakKeyDictionary):
            y = cls.__new__(cls)
            for k,v in x.items():
                y[k] = v
            return y
        if isinstance(x, _weakref.WeakValueDictionary):
            y = cls.__new__(cls)
            for k,v in x.items():
                y[k] = v
            return y
        y = cls.__new__(cls)
        if hasattr(x,'__dict__'):
            try: y.__dict__.update(x.__dict__)
            except: pass
        for k,v in x.items():
            try:
                dict.__setitem__(y, k, v)
            except Exception:
                try: y[k]=v
                except: pass
        return y
    if isinstance(x, set):
        try:
            y = cls.__new__(cls, x)
        except Exception:
            y = cls.__new__(cls)
            try: y.update(x)
            except: pass
        if hasattr(x,'__dict__'):
            try: y.__dict__.update(x.__dict__)
            except: pass
        slots = getattr(cls,'__slots__',None)
        if slots is not None:
            if isinstance(slots,str): slots=(slots,)
            for sl in slots:
                if isinstance(sl,str) and hasattr(x,sl):
                    try: setattr(y,sl,getattr(x,sl))
                    except: pass
        return y
    if isinstance(x, tuple):
        try:
            y = cls.__new__(cls, tuple(x))
        except Exception:
            try: y = cls(tuple(x))
            except: y = cls.__new__(cls)
        if hasattr(x,'__dict__'):
            try: y.__dict__.update(x.__dict__)
            except: pass
        slots = getattr(cls,'__slots__',None)
        if slots is not None:
            if isinstance(slots,str): slots=(slots,)
            for sl in slots:
                if isinstance(sl,str) and hasattr(x,sl):
                    try: setattr(y,sl,getattr(x,sl))
                    except: pass
        return y
    if isinstance(x, bytearray):
        try:
            y = cls(x)
        except Exception:
            y = cls.__new__(cls)
            try: y.extend(x)
            except: pass
        if hasattr(x,'__dict__'):
            try: y.__dict__.update(x.__dict__)
            except: pass
        return y
    g = getattr(x, "__getnewargs_ex__", None)
    if g is not None:
        try:
            args, kwargs = g()
            y = cls.__new__(cls, *args, **kwargs)
            gs = getattr(x, "__getstate__", None)
            if gs is not None:
                try: state = gs()
                except Exception: raise
                ss = getattr(y, "__setstate__", None)
                if ss is not None: ss(state)
                else:
                    if isinstance(state, dict): y.__dict__.update(state)
                    elif isinstance(state, tuple) and len(state)==2:
                        s, sl = state
                        if s is not None:
                            if isinstance(s, dict): y.__dict__.update(s)
                        if sl is not None:
                            for k,v in sl.items(): setattr(y,k,v)
                    elif state is not None:
                        try: y.__dict__.update(state)
                        except: pass
            else:
                if hasattr(x,'__dict__'): y.__dict__.update(x.__dict__)
                slots = getattr(cls,'__slots__',None)
                if slots is not None:
                    if isinstance(slots,str): slots=(slots,)
                    for sl in slots:
                        if isinstance(sl,str) and hasattr(x,sl): setattr(y,sl,getattr(x,sl))
            return y
        except Exception:
            pass
    g2 = getattr(x, "__getnewargs__", None)
    if g2 is not None:
        try:
            args = g2()
            y = cls.__new__(cls, *args)
            gs = getattr(x, "__getstate__", None)
            if gs is not None:
                try: state = gs()
                except Exception: raise
                ss = getattr(y, "__setstate__", None)
                if ss is not None: ss(state)
                else:
                    if isinstance(state, dict): y.__dict__.update(state)
                    elif isinstance(state, tuple) and len(state)==2:
                        s, sl = state
                        if s is not None:
                            if isinstance(s, dict): y.__dict__.update(s)
                        if sl is not None:
                            for k,v in sl.items(): setattr(y,k,v)
            else:
                if hasattr(x,'__dict__'): y.__dict__.update(x.__dict__)
                slots = getattr(cls,'__slots__',None)
                if slots is not None:
                    if isinstance(slots,str): slots=(slots,)
                    for sl in slots:
                        if isinstance(sl,str) and hasattr(x,sl): setattr(y,sl,getattr(x,sl))
            return y
        except Exception:
            pass
    g3 = getattr(x, "__getinitargs__", None)
    if g3 is not None:
        try:
            args = g3()
            y = cls.__new__(cls)
            try: y.__init__(*args)
            except: pass
            gs = getattr(x, "__getstate__", None)
            if gs is not None:
                try: state = gs()
                except Exception: raise
                ss = getattr(y, "__setstate__", None)
                if ss is not None: ss(state)
                else:
                    if isinstance(state, dict): y.__dict__.update(state)
            return y
        except Exception:
            pass
    gs = getattr(x, "__getstate__", None)
    if gs is not None:
        try: state = gs()
        except Exception: raise
        y = cls.__new__(cls)
        ss = getattr(y, "__setstate__", None)
        if ss is not None:
            ss(state)
        else:
            if isinstance(state, tuple) and len(state)==2:
                s, sl = state
            else:
                s, sl = state, None
            if s is not None:
                if isinstance(s, dict): y.__dict__.update(s)
                else:
                    try: y.__dict__.update(s)
                    except: pass
            if sl is not None:
                for k,v in sl.items(): setattr(y,k,v)
            if s is None and sl is None and hasattr(x,'__dict__'):
                y.__dict__.update(x.__dict__)
        return y
    if hasattr(x, "__setstate__"):
        y = cls.__new__(cls)
        if hasattr(x,'__dict__'): y.__dict__.update(x.__dict__)
        slots = getattr(cls,'__slots__',None)
        if slots is not None:
            if isinstance(slots,str): slots=(slots,)
            for sl in slots:
                if isinstance(sl,str) and hasattr(x,sl): setattr(y,sl,getattr(x,sl))
        return y
    y = cls.__new__(cls)
    if hasattr(x,'__dict__'):
        try: y.__dict__.update(x.__dict__)
        except: pass
    slots = getattr(cls,'__slots__',None)
    if slots is not None:
        if isinstance(slots,str): slots=(slots,)
        for sl in slots:
            if isinstance(sl,str) and hasattr(x,sl):
                try: setattr(y,sl,getattr(x,sl))
                except: pass
    return y

def copy(x):
    cls = type(x)
    if cls in _copy_atomic_types:
        return x
    if cls in _copy_builtin_containers:
        if cls is bytearray:
            return bytearray(x)
        return x.copy()
    # deque is not in _copy_builtin_containers but is copyable via x.copy()
    try:
        if getattr(cls, '__name__', None) == 'deque':
            try:
                return x.copy()
            except Exception:
                pass
    except Exception:
        pass
    if isinstance(x, type) or hasattr(x, '__mro__'):
        return x
    # also try instance-level __copy__ (deque's is instance-level)
    try:
        copier_inst = getattr(x, "__copy__", None)
        if copier_inst is not None:
            return copier_inst()
    except Exception:
        pass
    copier = getattr(cls, "__copy__", None)
    if copier is not None:
        return copier(x)
    reductor = dispatch_table.get(cls)
    if reductor is not None:
        rv = reductor(x)
    else:
        reductor = getattr(x, "__reduce_ex__", None)
        if reductor is not None:
            rv = reductor(4)
        else:
            reductor = getattr(x, "__reduce__", None)
            if reductor:
                rv = reductor()
            else:
                raise Error("un(shallow)copyable object of type %s" % cls)
    if rv is None:
        if _is_overridden_getattribute(cls):
            raise Error("un(shallow)copyable object of type %s" % cls)
        return _copy_shallow_fallback(x, cls)
    if isinstance(rv, str):
        return x
    return _reconstruct(x, None, *rv)

def _keep_alive(x, memo):
    try:
        memo[id(memo)].append(x)
    except KeyError:
        memo[id(memo)]=[x]

def _deepcopy_fallback(x, cls, memo, d):
    if isinstance(x, list):
        y = cls.__new__(cls)
        memo[d]=y; _keep_alive(x,memo)
        for item in x:
            y.append(deepcopy(item, memo))
        if hasattr(x,'__dict__'):
            for k,v in x.__dict__.items(): y.__dict__[k]=deepcopy(v,memo)
        slots = getattr(cls,'__slots__',None)
        if slots is not None:
            if isinstance(slots,str): slots=(slots,)
            for sl in slots:
                if isinstance(sl,str) and hasattr(x,sl): setattr(y,sl,deepcopy(getattr(x,sl),memo))
        return y
    if isinstance(x, dict):
        if isinstance(x, _weakref.WeakKeyDictionary):
            y = cls.__new__(cls)
            memo[d]=y; _keep_alive(x,memo)
            for k,v in x.items():
                y[k] = deepcopy(v, memo)
            # break reference to last loop variables (prevents keeping last key alive)
            try: del k, v
            except: pass
            return y
        if isinstance(x, _weakref.WeakValueDictionary):
            y = cls.__new__(cls)
            memo[d]=y; _keep_alive(x,memo)
            for k,v in x.items():
                y[deepcopy(k,memo)] = v
            try: del k, v
            except: pass
            return y
        y = cls.__new__(cls)
        memo[d]=y; _keep_alive(x,memo)
        if hasattr(x,'__dict__'):
            for k2,v2 in x.__dict__.items(): y.__dict__[k2]=deepcopy(v2,memo)
        for k,v in x.items():
            try:
                dict.__setitem__(y, deepcopy(k,memo), deepcopy(v,memo))
            except Exception:
                try: y[deepcopy(k,memo)] = deepcopy(v,memo)
                except: pass
        slots = getattr(cls,'__slots__',None)
        if slots is not None:
            if isinstance(slots,str): slots=(slots,)
            for sl in slots:
                if isinstance(sl,str) and hasattr(x,sl): setattr(y,sl,deepcopy(getattr(x,sl),memo))
        return y
    if isinstance(x, set):
        y = cls.__new__(cls)
        memo[d]=y; _keep_alive(x,memo)
        for item in x:
            y.add(deepcopy(item,memo))
        if hasattr(x,'__dict__'):
            for k,v in x.__dict__.items(): y.__dict__[k]=deepcopy(v,memo)
        return y
    if isinstance(x, tuple):
        lst = [deepcopy(a,memo) for a in x]
        for k,j in zip(x,lst):
            if k is not j:
                y = cls.__new__(cls, tuple(lst))
                memo[d]=y; _keep_alive(x,memo)
                if hasattr(x,'__dict__'):
                    for k2,v in x.__dict__.items(): y.__dict__[k2]=deepcopy(v,memo)
                return y
        y = x
        memo[d]=y; _keep_alive(x,memo)
        return y
    if isinstance(x, bytearray):
        y = cls(x)
        memo[d]=y; _keep_alive(x,memo)
        return y
    g = getattr(x, "__getnewargs_ex__", None)
    if g is not None:
        args, kwargs = g()
        args_d = tuple(deepcopy(a, memo) for a in args) if args else ()
        kwargs_d = {k: deepcopy(v, memo) for k,v in kwargs.items()} if kwargs else {}
        y = cls.__new__(cls, *args_d, **kwargs_d)
        memo[d]=y; _keep_alive(x,memo)
        gs = getattr(x, "__getstate__", None)
        if gs is not None:
            try: state = gs()
            except: raise
            state_d = deepcopy(state,memo)
            ss = getattr(y, "__setstate__", None)
            if ss is not None: ss(state_d)
            else:
                if isinstance(state_d, tuple) and len(state_d)==2:
                    sd, sl = state_d
                else:
                    sd, sl = state_d, None
                if sd is not None:
                    if isinstance(sd, dict):
                        for k,v in sd.items(): y.__dict__[k]=v
                    else:
                        y.__dict__.update(sd)
                if sl is not None:
                    for k,v in sl.items(): setattr(y,k,v)
        else:
            if hasattr(x,'__dict__'):
                for k,v in x.__dict__.items(): y.__dict__[k]=deepcopy(v,memo)
            slots = getattr(cls,'__slots__',None)
            if slots is not None:
                if isinstance(slots,str): slots=(slots,)
                for sl in slots:
                    if isinstance(sl,str) and hasattr(x,sl): setattr(y,sl,deepcopy(getattr(x,sl),memo))
        return y
    g2 = getattr(x, "__getnewargs__", None)
    if g2 is not None:
        args = g2()
        args_d = tuple(deepcopy(a,memo) for a in args)
        y = cls.__new__(cls, *args_d)
        memo[d]=y; _keep_alive(x,memo)
        gs = getattr(x, "__getstate__", None)
        if gs is not None:
            try: state = gs()
            except: raise
            state_d = deepcopy(state,memo)
            ss = getattr(y, "__setstate__", None)
            if ss is not None: ss(state_d)
            else:
                if isinstance(state_d, tuple) and len(state_d)==2:
                    sd, sl = state_d
                else:
                    sd, sl = state_d, None
                if sd is not None:
                    if isinstance(sd, dict):
                        for k,v in sd.items(): y.__dict__[k]=v
                    else:
                        y.__dict__.update(sd)
                if sl is not None:
                    for k,v in sl.items(): setattr(y,k,v)
        else:
            if hasattr(x,'__dict__'):
                for k,v in x.__dict__.items(): y.__dict__[k]=deepcopy(v,memo)
            slots = getattr(cls,'__slots__',None)
            if slots is not None:
                if isinstance(slots,str): slots=(slots,)
                for sl in slots:
                    if isinstance(sl,str) and hasattr(x,sl): setattr(y,sl,deepcopy(getattr(x,sl),memo))
        return y
    g3 = getattr(x, "__getinitargs__", None)
    if g3 is not None:
        args = g3()
        args_d = tuple(deepcopy(a,memo) for a in args)
        y = cls.__new__(cls)
        memo[d]=y; _keep_alive(x,memo)
        try: y.__init__(*args_d)
        except: pass
        gs = getattr(x, "__getstate__", None)
        if gs is not None:
            try: state = gs()
            except: raise
            state_d = deepcopy(state,memo)
            ss = getattr(y, "__setstate__", None)
            if ss is not None: ss(state_d)
            else:
                if isinstance(state_d, dict): y.__dict__.update(state_d)
        return y
    y = cls.__new__(cls)
    memo[d]=y; _keep_alive(x,memo)
    gstate = getattr(x, "__getstate__", None)
    if gstate is not None:
        try: state = gstate()
        except: raise
        s_set = getattr(y, "__setstate__", None)
        if s_set is not None:
            s_set(deepcopy(state,memo))
        else:
            if isinstance(state, tuple) and len(state)==2:
                state, slotstate = state
            else:
                slotstate = None
            if state is not None:
                if isinstance(state, dict):
                    for k,v in state.items(): y.__dict__[k]=deepcopy(v,memo)
                else:
                    y.__dict__.update(deepcopy(state,memo))
            if slotstate is not None:
                for k,v in slotstate.items(): setattr(y,k,deepcopy(v,memo))
    else:
        if hasattr(x,'__dict__'):
            for k,v in x.__dict__.items(): y.__dict__[k]=deepcopy(v,memo)
        slots = getattr(cls,'__slots__',None)
        if slots is not None:
            if isinstance(slots,str): slots=(slots,)
            for sl in slots:
                if isinstance(sl,str) and hasattr(x,sl): setattr(y,sl,deepcopy(getattr(x,sl),memo))
    return y

def deepcopy(x, memo=None, _nil=[]):
    cls = type(x)
    if cls in _atomic_types:
        return x
    d = id(x)
    if memo is None:
        memo = {}
    else:
        y = memo.get(d, _nil)
        if y is not _nil:
            return y
    # deque fast path
    try:
        if getattr(cls, '__name__', None) == 'deque':
            from copy import deepcopy as _dc
            try:
                y = type(x)([deepcopy(a, memo) for a in x], maxlen=getattr(x, 'maxlen', None))
                memo[d] = y
                return y
            except Exception:
                pass
    except Exception:
        pass
    copier = _deepcopy_dispatch.get(cls)
    if copier is not None:
        y = copier(x, memo)
    else:
        if isinstance(x, type) or hasattr(x, '__mro__'):
            y = x
        else:
            # instance-level __deepcopy__
            try:
                copier_i = getattr(x, "__deepcopy__", None)
                if copier_i is not None:
                    y = copier_i(memo)
                    memo[d] = y
                    return y
            except Exception:
                pass
            copier = getattr(x, "__deepcopy__", None)
            if copier is not None:
                y = copier(memo)
            else:
                reductor = dispatch_table.get(cls)
                if reductor:
                    rv = reductor(x)
                else:
                    reductor = getattr(x, "__reduce_ex__", None)
                    if reductor is not None:
                        rv = reductor(4)
                    else:
                        reductor = getattr(x, "__reduce__", None)
                        if reductor:
                            rv = reductor()
                        else:
                            raise Error("un(deep)copyable object of type %s" % cls)
                if rv is None:
                    if _is_overridden_getattribute(cls):
                        raise Error("un(deep)copyable object of type %s" % cls)
                    y = _deepcopy_fallback(x, cls, memo, d)
                    if y is not x:
                        return y
                elif isinstance(rv, str):
                    y = x
                else:
                    y = _reconstruct(x, memo, *rv)
    if y is not x:
        memo[d] = y
        _keep_alive(x, memo)
    return y

_atomic_types =  {types.NoneType, _EllipsisType, _NotImplementedType,
          int, float, bool, complex, bytes, str, _CodeType, type, range,
          _BuiltinFunctionType, _FunctionType, weakref.ref, property}
try:
    _atomic_types = _atomic_types | _atomic_types_for_shim
except: pass

_deepcopy_dispatch = d = {}

def _deepcopy_list(x, memo, deepcopy=deepcopy):
    y = []
    memo[id(x)] = y
    append = y.append
    for a in x:
        append(deepcopy(a, memo))
    return y
d[list] = _deepcopy_list

def _deepcopy_tuple(x, memo, deepcopy=deepcopy):
    y = [deepcopy(a, memo) for a in x]
    try:
        return memo[id(x)]
    except KeyError:
        pass
    for k, j in zip(x, y):
        if k is not j:
            y = tuple(y)
            break
    else:
        y = x
    return y
d[tuple] = _deepcopy_tuple

def _deepcopy_dict(x, memo, deepcopy=deepcopy):
    y = {}
    memo[id(x)] = y
    for key, value in x.items():
        y[deepcopy(key, memo)] = deepcopy(value, memo)
    return y
d[dict] = _deepcopy_dict

def _deepcopy_method(x, memo):
    return _types.MethodType(x.__func__, deepcopy(x.__self__, memo))
d[types.MethodType] = _deepcopy_method
try:
    class _Probe:
        def _m(self): pass
    _real_mtype = type(_Probe()._m)
    d[_real_mtype] = _deepcopy_method
except: pass

del d

# Keep aliases for use after `del types, weakref` below (fallback helpers need them at call time)
_types = types
_weakref = weakref

def _reconstruct(x, memo, func, args,
                 state=None, listiter=None, dictiter=None,
                 *, deepcopy=deepcopy):
    deep = memo is not None
    if deep and args:
        args = (deepcopy(arg, memo) for arg in args)
    y = func(*args)
    if deep:
        memo[id(x)] = y
    if state is not None:
        if deep:
            state = deepcopy(state, memo)
        if hasattr(y, '__setstate__'):
            y.__setstate__(state)
        else:
            if isinstance(state, tuple) and len(state) == 2:
                state, slotstate = state
            else:
                slotstate = None
            if state is not None:
                y.__dict__.update(state)
            if slotstate is not None:
                for key, value in slotstate.items():
                    setattr(y, key, value)
    if listiter is not None:
        if deep:
            for item in listiter:
                item = deepcopy(item, memo)
                y.append(item)
        else:
            for item in listiter:
                y.append(item)
    if dictiter is not None:
        if deep:
            for key, value in dictiter:
                key = deepcopy(key, memo)
                value = deepcopy(value, memo)
                y[key] = value
        else:
            for key, value in dictiter:
                y[key] = value
    return y

del types, weakref

def replace(obj, /, **changes):
    cls = obj.__class__
    # Early validation for namedtuple field names (for Point etc.)
    fields = None
    if hasattr(obj, '_fields'):
        fields = obj._fields
    elif hasattr(type(obj), '_fields'):
        fields = type(obj)._fields
    elif hasattr(type(obj), '__annotations__') and type(obj).__annotations__:
        # For typing.NamedTuple like PointFromClass, check if obj has args
        if 'args' in getattr(obj, '__dict__', {}):
            fields = list(type(obj).__annotations__.keys())
    if fields is not None:
        for k in changes:
            if k not in fields:
                raise TypeError(f"unexpected field name '{k}'")
    func = getattr(cls, '__replace__', None)
    if func is not None:
        return func(obj, **changes)
    if hasattr(obj, '_replace'):
        try:
            return obj._replace(**changes)
        except TypeError:
            raise
    if hasattr(obj, '_fields') or hasattr(type(obj), '_fields'):
        fields = getattr(obj, '_fields', None) or getattr(type(obj), '_fields', None)
        if fields is not None:
            for k in changes:
                if k not in fields:
                    raise TypeError(f"unexpected field name '{k}'")
        if hasattr(obj, '_replace'):
            try:
                return obj._replace(**changes)
            except TypeError:
                raise
        try:
            d = obj._asdict()
        except Exception:
            try:
                d = dict(zip(fields, obj))
            except Exception:
                d = {}
        d.update(changes)
        if hasattr(type(obj), '_make'):
            try:
                return type(obj)._make(d.values())
            except Exception:
                pass
        try:
            return type(obj)(*list(d.values()))
        except Exception:
            try:
                return type(obj)(**d)
            except Exception:
                pass
    # Fallback for typing.NamedTuple (e.g., PointFromClass) via __annotations__ and __dict__['args']
    if hasattr(type(obj), '__annotations__') and 'args' in getattr(obj, '__dict__', {}):
        fields = list(type(obj).__annotations__.keys())
        for k in changes:
            if k not in fields:
                raise TypeError(f"unexpected field name '{k}'")
        d = dict(zip(fields, obj.__dict__['args']))
        d.update(changes)
        try:
            return type(obj)(*list(d.values()))
        except:
            return type(obj)(**d)
    try:
        import dataclasses
        if dataclasses.is_dataclass(obj) and not isinstance(obj, type):
            return dataclasses.replace(obj, **changes)
    except ImportError:
        pass
    raise TypeError(f"replace() does not support {cls.__name__} objects")


